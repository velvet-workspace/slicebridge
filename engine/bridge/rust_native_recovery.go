package bridge

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const (
	rustScratchMinAge = 5 * time.Minute // longer than the 120s Rust conversion limit
	rustJobMarker     = ".slicebridge-owner.json"
	rustWorkerMarker  = ".slicebridge-worker.pid"
	rustSpawnMarker   = ".slicebridge-spawn-intent"
)

type rustJobOwner struct {
	Version   int    `json:"version"`
	PID       int    `json:"pid"`
	Directory string `json:"directory"`
}

type rustSnapshotOwner struct {
	Version  int    `json:"version"`
	PID      int    `json:"pid"`
	Filename string `json:"filename"`
}

func writeRustSnapshotOwner(path string) error {
	marker, err := json.Marshal(rustSnapshotOwner{Version: 1, PID: os.Getpid(), Filename: filepath.Base(path)})
	if err != nil {
		return err
	}
	return os.WriteFile(path+".owner", marker, 0600)
}

func newRustNativeJob(outDir string) (string, error) {
	job, err := os.MkdirTemp(outDir, fmt.Sprintf(".bridge-rust-native-job-p%d-*", os.Getpid()))
	if err != nil {
		return "", err
	}
	owner, err := json.Marshal(rustJobOwner{Version: 1, PID: os.Getpid(), Directory: filepath.Base(job)})
	if err == nil {
		err = os.WriteFile(filepath.Join(job, rustJobMarker), owner, 0600)
	}
	if err != nil {
		_ = os.RemoveAll(job)
		return "", err
	}
	return job, nil
}

func rustNativeWorkerSpawnAttempted(job string) error {
	return os.WriteFile(filepath.Join(job, rustSpawnMarker), []byte("spawn-intent-v1"), 0600)
}

func rustNativeWorkerStarted(job string, pid int) error {
	if pid <= 0 {
		return fmt.Errorf("invalid Rust child pid")
	}
	return os.WriteFile(filepath.Join(job, rustWorkerMarker), []byte(strconv.Itoa(pid)+"\n"), 0600)
}

// PID-based reclamation is deliberately limited to scratch files created by
// this bridge version in the selected destination directory, never a user's
// 3MF, arbitrary hidden directory or another live app's unfinished job.
// Older unmarked scratch is not guessed to be ours.
func recoverRustNativeScratch(outDir string) (int, error) {
	entries, err := os.ReadDir(outDir)
	if err != nil {
		return 0, err
	}
	recovered := 0
	now := time.Now()
	for _, entry := range entries {
		name := entry.Name()
		full := filepath.Join(outDir, name)
		if strings.HasPrefix(name, ".bridge-rust-source-p") && strings.HasSuffix(name, ".3mf") {
			pid, ok := rustScratchOwnerPID(name, ".bridge-rust-source-p", ".3mf")
			if !ok || rustProcessAlive(pid) || rustOwnerHasRunningWorker(outDir, pid, entries) {
				continue
			}
			info, err := os.Lstat(full)
			if err != nil || !info.Mode().IsRegular() || now.Sub(info.ModTime()) < rustScratchMinAge {
				continue
			}
			marker := full + ".owner"
			markInfo, err := os.Lstat(marker)
			if err != nil || !markInfo.Mode().IsRegular() {
				continue
			}
			b, err := os.ReadFile(marker)
			if err != nil || len(b) > 512 {
				continue
			}
			var owner rustSnapshotOwner
			if json.Unmarshal(b, &owner) != nil || owner.Version != 1 || owner.PID != pid || owner.Filename != name {
				continue
			}
			if err := os.Remove(full); err != nil {
				continue
			}
			_ = os.Remove(marker)
			recovered++
			continue
		}
		if !strings.HasPrefix(name, ".bridge-rust-native-job-p") {
			continue
		}
		pid, ok := rustScratchOwnerPID(name, ".bridge-rust-native-job-p", "")
		if !ok || rustProcessAlive(pid) {
			continue
		}
		info, err := os.Lstat(full)
		if err != nil || !info.IsDir() || now.Sub(info.ModTime()) < rustScratchMinAge {
			continue
		}
		b, err := os.ReadFile(filepath.Join(full, rustJobMarker))
		if err != nil || len(b) > 512 {
			continue
		}
		var owner rustJobOwner
		if json.Unmarshal(b, &owner) != nil || owner.Version != 1 ||
			owner.PID != pid || owner.Directory != name {
			continue
		}
		worker, err := os.ReadFile(filepath.Join(full, rustWorkerMarker))
		if err == nil {
			child, e := strconv.Atoi(strings.TrimSpace(string(worker)))
			if e != nil || child <= 0 || rustProcessAlive(child) {
				continue
			}
		} else if !errors.Is(err, os.ErrNotExist) {
			continue
		} else if _, err := os.Lstat(filepath.Join(full, rustSpawnMarker)); err == nil {
			// Parent vanished between spawning and recording the child's
			// PID. Its liveness cannot be proven, so don't delete the job.
			continue
		} else if !errors.Is(err, os.ErrNotExist) {
			continue
		}
		// Reclaim ONLY a bounded set of exact regular files and Rust's private
		// partial archive names. A surprise file, subdirectory or symlink
		// makes the entire job ineligible for deletion.
		files, err := os.ReadDir(full)
		if err != nil || len(files) > 12 {
			continue
		}
		safe := true
		for _, f := range files {
			filename := f.Name()
			isKnown := filename == rustJobMarker || filename == rustWorkerMarker || filename == rustSpawnMarker ||
				filename == "verified-request.json" || filename == "converted.3mf" ||
				(strings.HasPrefix(filename, ".slicebridge-") && strings.HasSuffix(filename, ".partial"))
			fi, err := os.Lstat(filepath.Join(full, filename))
			if err != nil || !fi.Mode().IsRegular() || !isKnown {
				safe = false
				break
			}
		}
		if !safe {
			continue
		}
		// Exclusive owner pid is gone; all contents were verified above.
		if err := os.RemoveAll(full); err == nil {
			recovered++
		}
	}
	return recovered, nil
}

func rustScratchOwnerPID(name, prefix, suffix string) (int, bool) {
	if !strings.HasPrefix(name, prefix) || !strings.HasSuffix(name, suffix) {
		return 0, false
	}
	middle := strings.TrimSuffix(strings.TrimPrefix(name, prefix), suffix)
	parts := strings.SplitN(middle, "-", 2)
	if len(parts) != 2 || len(parts[1]) < 5 || len(parts[1]) > 64 {
		return 0, false
	}
	for _, r := range parts[1] {
		if !((r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9')) {
			return 0, false
		}
	}
	pid, err := strconv.Atoi(parts[0])
	return pid, err == nil && pid > 0 && pid <= 2147483647
}

// A dead parent may have left its child running. Keep its frozen input
// available until the child has exited, even after the age threshold.
func rustOwnerHasRunningWorker(outDir string, ownerPID int, entries []os.DirEntry) bool {
	for _, entry := range entries {
		if !entry.IsDir() || !strings.HasPrefix(entry.Name(), ".bridge-rust-native-job-p") {
			continue
		}
		pid, ok := rustScratchOwnerPID(entry.Name(), ".bridge-rust-native-job-p", "")
		if !ok || pid != ownerPID {
			continue
		}
		data, err := os.ReadFile(filepath.Join(outDir, entry.Name(), rustWorkerMarker))
		if err == nil {
			child, conversionErr := strconv.Atoi(strings.TrimSpace(string(data)))
			if conversionErr != nil || child <= 0 || rustProcessAlive(child) {
				return true
			}
		} else if !errors.Is(err, os.ErrNotExist) {
			return true
		}
	}
	return false
}
