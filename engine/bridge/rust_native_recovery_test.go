package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func writeTestRustJob(t *testing.T, dir string, pid int, suffix string, workerPID int, extra string) string {
	t.Helper()
	path := filepath.Join(dir, fmt.Sprintf(".bridge-rust-native-job-p%d-%s", pid, suffix))
	if err := os.Mkdir(path, 0700); err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(rustJobOwner{Version: 1, PID: pid, Directory: filepath.Base(path)})
	if err := os.WriteFile(filepath.Join(path, rustJobMarker), b, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(path, "verified-request.json"), []byte("{}"), 0600); err != nil {
		t.Fatal(err)
	}
	if workerPID > 0 {
		if err := rustNativeWorkerStarted(path, workerPID); err != nil {
			t.Fatal(err)
		}
	}
	if extra != "" {
		if err := os.WriteFile(filepath.Join(path, extra), []byte("not a bridge file"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	old := time.Now().Add(-10 * time.Minute)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatal(err)
	}
	return path
}

func writeTestRustSnapshot(t *testing.T, dir string, pid int, suffix string, marker bool) string {
	t.Helper()
	path := filepath.Join(dir, fmt.Sprintf(".bridge-rust-source-p%d-%s.3mf", pid, suffix))
	if err := os.WriteFile(path, []byte("private frozen input"), 0600); err != nil {
		t.Fatal(err)
	}
	if marker {
		b, _ := json.Marshal(rustSnapshotOwner{Version: 1, PID: pid, Filename: filepath.Base(path)})
		if err := os.WriteFile(path+".owner", b, 0600); err != nil {
			t.Fatal(err)
		}
	}
	old := time.Now().Add(-10 * time.Minute)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestRustNativeRecoveryRemovesOnlyDeadOwnedScratch(t *testing.T) {
	dir := t.TempDir()
	const deadPID = 2147483000
	deadJob := writeTestRustJob(t, dir, deadPID, "deadownerabc", 0, "")
	deadSource := writeTestRustSnapshot(t, dir, deadPID, "deadownerabc", true)
	liveJob := writeTestRustJob(t, dir, os.Getpid(), "liveownerabc", 0, "")
	liveSource := writeTestRustSnapshot(t, dir, os.Getpid(), "liveownerabc", true)
	foreignJob := writeTestRustJob(t, dir, deadPID, "foreignfilesabc", 0, "user-file.txt")
	unmarkedSource := writeTestRustSnapshot(t, dir, deadPID, "unmarkedabcd", false)
	liveWorkerJob := writeTestRustJob(t, dir, deadPID, "liveworkerabc", os.Getpid(), "")
	uncertainSpawn := writeTestRustJob(t, dir, deadPID, "spawnunknownabc", 0, "")
	if err := rustNativeWorkerSpawnAttempted(uncertainSpawn); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-10 * time.Minute)
	if err := os.Chtimes(uncertainSpawn, old, old); err != nil {
		t.Fatal(err)
	}
	// Conservatively leave the source when its dead parent's Rust worker is live.
	workerSource := writeTestRustSnapshot(t, dir, deadPID, "liveworkerabc", true)
	// Only the two verified, dead and aged resources may be removed. Check
	// this independently from any published 3MF owned by a user.
	published := filepath.Join(dir, "published.3mf")
	bytesBefore := []byte("the user's published archive is immutable")
	if err := os.WriteFile(published, bytesBefore, 0600); err != nil {
		t.Fatal(err)
	}
	got, err := recoverRustNativeScratch(dir)
	if err != nil {
		t.Fatal(err)
	}
	// Both resources share the same dead PID as an active worker; retaining
	// the source is required, and the other job is still independently stale.
	if got != 1 {
		t.Fatalf("expected one eligible stale job, got %d", got)
	}
	for _, path := range []string{deadJob} {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatalf("dead scratch remained: %s: %v", path, err)
		}
	}
	for _, path := range []string{deadSource, liveJob, liveSource, foreignJob, unmarkedSource, liveWorkerJob, uncertainSpawn, workerSource} {
		if _, err := os.Lstat(path); err != nil {
			t.Fatalf("unverified or live resource was removed: %s: %v", path, err)
		}
	}
	after, err := os.ReadFile(published)
	if err != nil || !bytes.Equal(after, bytesBefore) {
		t.Fatalf("published 3MF changed: %v", err)
	}
}

func TestRustNativeReclaimsOrphanedParentAfterGracePeriod(t *testing.T) {
	// This subprocess models an app parent dying with private scratch but
	// does NOT model a live Rust child: true child lifetime tracking is
	// tested separately by the owner/worker PID guards above.
	if os.Getenv("SLICEBRIDGE_TEST_RUST_ORPHAN_CHILD") == "1" {
		dir := os.Getenv("SLICEBRIDGE_TEST_RUST_ORPHAN_DIR")
		job, err := newRustNativeJob(dir)
		if err != nil {
			t.Fatal(err)
		}
		source, err := os.CreateTemp(dir, fmt.Sprintf(".bridge-rust-source-p%d-*.3mf", os.Getpid()))
		if err != nil {
			t.Fatal(err)
		}
		path := source.Name()
		if _, err = source.Write([]byte("private source")); err != nil {
			t.Fatal(err)
		}
		if err = source.Close(); err != nil {
			t.Fatal(err)
		}
		if err = writeRustSnapshotOwner(path); err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(dir, "ready"), []byte(job+"\n"+path), 0600); err != nil {
			t.Fatal(err)
		}
		time.Sleep(30 * time.Second)
		return
	}
	dir := t.TempDir()
	cmd := exec.Command(os.Args[0], "-test.run=^TestRustNativeReclaimsOrphanedParentAfterGracePeriod$")
	cmd.Env = append(os.Environ(), "SLICEBRIDGE_TEST_RUST_ORPHAN_CHILD=1", "SLICEBRIDGE_TEST_RUST_ORPHAN_DIR="+dir)
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()
	var paths []string
	deadline := time.After(8 * time.Second)
	for len(paths) != 2 {
		data, err := os.ReadFile(filepath.Join(dir, "ready"))
		if err == nil {
			paths = strings.Split(strings.TrimSpace(string(data)), "\n")
		}
		if len(paths) == 2 {
			break
		}
		select {
		case <-deadline:
			t.Fatal("test parent never produced scratch ownership markers")
		default:
			time.Sleep(10 * time.Millisecond)
		}
	}
	if err := cmd.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	_ = cmd.Wait()
	old := time.Now().Add(-10 * time.Minute)
	for _, path := range paths {
		if err := os.Chtimes(path, old, old); err != nil {
			t.Fatal(err)
		}
	}
	count, err := recoverRustNativeScratch(dir)
	if err != nil {
		t.Fatal(err)
	}
	if count != 2 {
		t.Fatalf("expected dead app parent job and snapshot reclaimed, got %d", count)
	}
	for _, path := range paths {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatalf("dead app scratch remains %s: %v", path, err)
		}
	}
}

func TestRustNativeNextAppConversionReclaimsDeadOwnerWithoutTouchingOutput(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	oem := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if binary == "" || oem == "" {
		t.Skip("requires an approved Rust binary and read-only installed OEM snapshots")
	}
	dir := t.TempDir()
	const deadPID = 2147482998
	staleJob := writeTestRustJob(t, dir, deadPID, "deadapprestart", 0, "")
	staleSource := writeTestRustSnapshot(t, dir, deadPID, "deadapprestart", true)
	original, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", "bambu-five-colors.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	src := filepath.Join(dir, "source.3mf")
	if err := os.WriteFile(src, original, 0600); err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(dir, "converted.3mf")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	report, err := ConvertTargetContext(context.Background(), src, dst, TargetRequest{
		Target: "u1", Nozzle: "0.4", Engine: "rust-native",
		ProfileFile: filepath.Join(oem, "scratch-u1-0.4-resolved.json"),
	})
	if err != nil || report["engine"] != "rust-native-3mf" {
		t.Fatalf("post-restart app conversion failed: %+v %v", report, err)
	}
	for _, path := range []string{staleJob, staleSource, staleSource + ".owner"} {
		if _, err := os.Lstat(path); !os.IsNotExist(err) {
			t.Fatalf("app conversion did not clean dead owner scratch %s: %v", path, err)
		}
	}
	after, err := os.ReadFile(src)
	if err != nil || !bytes.Equal(original, after) {
		t.Fatal("restart cleanup changed the source")
	}
	output, err := os.Stat(dst)
	if err != nil || output.Size() == 0 {
		t.Fatalf("recovery did not publish valid new 3MF: %v", err)
	}
	remaining, err := filepath.Glob(filepath.Join(dir, ".bridge-*"))
	if err != nil || len(remaining) != 0 {
		t.Fatalf("restart leaked scratch: %v (%v)", remaining, err)
	}
}
