package bridge

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

// freezeRustSourceForPlan binds production Go's profile/filament/nozzle plan
// and Rust's 3MF transformation to ONE immutable private source snapshot.
// Without this, an in-place editor or path replacement between buildPlan and
// Rust's own source capture could produce an output from different geometry.
// This function is used ONLY in the experimental Rust opt-in path.
//
// The original file is never opened for writing, and the private source is
// removed by the caller on success or error. A killed process can leave
// an orphaned snapshot; the next Rust-native conversion in the same output
// directory reclaims verified dead-owner scratch after a conservative grace
// period, while unknown files and active workers are never reclaimed.
func freezeRustSourceForPlan(source, destination string) (string, func(), error) {
	input, err := os.Open(source)
	if err != nil {
		return "", nil, err
	}
	defer input.Close()
	info, err := input.Stat()
	if err != nil {
		return "", nil, err
	}
	if !info.Mode().IsRegular() || info.Size() == 0 || info.Size() > maxCompressedBytes {
		return "", nil, fmt.Errorf("Rust input must be a nonempty regular 3MF up to 250 MiB")
	}
	parent := filepath.Dir(destination)
	dirInfo, err := os.Stat(parent)
	if err != nil || !dirInfo.IsDir() {
		return "", nil, fmt.Errorf("Rust output directory does not exist: %s", parent)
	}
	temp, err := os.CreateTemp(parent, fmt.Sprintf(".bridge-rust-source-p%d-*.3mf", os.Getpid()))
	if err != nil {
		return "", nil, err
	}
	path := temp.Name()
	cleanup := func() {
		_ = os.Remove(path)
		_ = os.Remove(path + ".owner")
	}
	success := false
	defer func() {
		_ = temp.Close()
		if !success {
			cleanup()
		}
	}()
	if err := writeRustSnapshotOwner(path); err != nil {
		return "", nil, fmt.Errorf("cannot identify private Rust snapshot: %w", err)
	}
	// Bound the copy independently of unreliable or stale file metadata.
	// Hash bytes as they are captured; verify by rereading the original open
	// descriptor, detecting same-sized in-place edits during the copy.
	first := sha256.New()
	n, err := io.Copy(io.MultiWriter(temp, first), io.LimitReader(input, maxCompressedBytes+1))
	if err != nil {
		return "", nil, fmt.Errorf("Rust source snapshot: %w", err)
	}
	if n != info.Size() || n > maxCompressedBytes {
		return "", nil, fmt.Errorf("source changed length during Rust snapshot capture")
	}
	if err := temp.Sync(); err != nil {
		return "", nil, err
	}
	if _, err := input.Seek(0, io.SeekStart); err != nil {
		return "", nil, err
	}
	second := sha256.New()
	rechecked, err := io.Copy(second, io.LimitReader(input, maxCompressedBytes+1))
	if err != nil {
		return "", nil, fmt.Errorf("Rust source snapshot verification: %w", err)
	}
	if rechecked != n || !bytes.Equal(first.Sum(nil), second.Sum(nil)) {
		return "", nil, fmt.Errorf("source changed bytes during Rust snapshot capture")
	}
	now, err := os.Stat(source)
	if err != nil || !os.SameFile(info, now) {
		return "", nil, fmt.Errorf("source path was replaced during Rust snapshot capture")
	}
	if err := temp.Close(); err != nil {
		return "", nil, err
	}
	success = true
	return path, cleanup, nil
}
