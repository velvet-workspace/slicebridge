package bridge

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
	"time"
)

// A deliberately controlled local helper represents a Rust worker that
// crashes or hangs AFTER starting, without any slicing or machine commands.
// The test checks that the actual app engine kills the child on cancellation,
// removes scratch files, and never publishes a partially converted archive.
func TestRustNativeCrashedWorkerAndInFlightCancelFailClosed(t *testing.T) {
	oemRoot := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if oemRoot == "" {
		t.Skip("requires the approved, read-only installed OEM snapshot")
	}
	if _, err := exec.LookPath("go"); err != nil {
		t.Skip("requires a local Go toolchain to compile the test-only worker")
	}
	directory := t.TempDir()
	workerSource := filepath.Join(directory, "controlled_worker.go")
	workerBinary := filepath.Join(directory, "controlled-worker")
	if runtime.GOOS == "windows" {
		workerBinary += ".exe"
	}
	const workerProgram = `package main
import ("os"; "time")
func main() {
	if path := os.Getenv("SLICEBRIDGE_FAKE_READY"); path != "" {
		_ = os.WriteFile(path, []byte("started"), 0600)
	}
	if os.Getenv("SLICEBRIDGE_FAKE_MODE") == "hang" {
		time.Sleep(30*time.Second)
	}
	os.Exit(42)
}`
	if err := os.WriteFile(workerSource, []byte(workerProgram), 0600); err != nil {
		t.Fatal(err)
	}
	compile, stopBuild := context.WithTimeout(context.Background(), 60*time.Second)
	defer stopBuild()
	cmd := exec.CommandContext(compile, "go", "build", "-o", workerBinary, workerSource)
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("cannot build controlled local test worker: %v (%s)", err, output)
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", workerBinary)
	src := filepath.Join(directory, "source.3mf")
	original, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(src, original, 0600); err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(directory, "must-not-publish.3mf")
	request := TargetRequest{
		Target: "qidi", Nozzle: "0.4", Engine: "rust-native",
		ProfileFile: filepath.Join(oemRoot, "scratch-qidi-0.4-resolved.json"),
	}
	ready := filepath.Join(directory, "worker-started")
	t.Setenv("SLICEBRIDGE_FAKE_READY", ready)
	t.Setenv("SLICEBRIDGE_FAKE_MODE", "hang")
	ctx, cancel := context.WithCancel(context.Background())
	result := make(chan error, 1)
	go func() {
		_, err := ConvertTargetContext(ctx, src, dst, request)
		result <- err
	}()
	poll := time.NewTicker(10 * time.Millisecond)
	defer poll.Stop()
	startLimit := time.NewTimer(5 * time.Second)
	defer startLimit.Stop()
started:
	for {
		if _, err := os.Stat(ready); err == nil {
			break started
		}
		select {
		case err := <-result:
			cancel()
			t.Fatalf("worker stopped before in-flight cancellation: %v", err)
		case <-startLimit.C:
			cancel()
			t.Fatal("controlled child worker never started")
		case <-poll.C:
		}
	}
	cancel()
	select {
	case err := <-result:
		if err == nil || ErrorCode(err) != "RUST_ENGINE_FAILED" {
			t.Fatalf("canceled in-flight Rust worker did not fail closed: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("canceled Rust worker was not reaped")
	}
	assertClean := func(stage string) {
		t.Helper()
		if _, err := os.Stat(dst); !os.IsNotExist(err) {
			t.Fatalf("%s published partial output: %v", stage, err)
		}
		after, err := os.ReadFile(src)
		if err != nil || !bytes.Equal(after, original) {
			t.Fatalf("%s modified original input", stage)
		}
		leftovers, err := filepath.Glob(filepath.Join(directory, ".bridge-*"))
		if err != nil || len(leftovers) != 0 {
			t.Fatalf("%s leaked conversion scratch: %v (%v)", stage, leftovers, err)
		}
	}
	assertClean("in-flight cancel")

	// Same source/target after the killed worker: a child crash must be
	// surfaced as an error, without fallback to Go or any output publication.
	t.Setenv("SLICEBRIDGE_FAKE_MODE", "crash")
	_, err = ConvertTargetContext(context.Background(), src, dst, request)
	if err == nil || ErrorCode(err) != "RUST_ENGINE_FAILED" {
		t.Fatalf("crashed Rust worker did not fail closed: %v", err)
	}
	assertClean("worker crash")
}
