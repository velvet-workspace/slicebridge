package bridge

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

// The Go parent owns one private per-invocation Rust workspace. Verify
// cleanup on success, failed child validation and exclusive output refusal.
// This is opt-in and never reads installed presets or touches real printers.
func TestRustGoParentCleansPrivateJobDirectories(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set an explicit experimental Rust executable")
	}
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", rust)
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	fixture(t, source, 2)
	before, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	noJobs := func(label string) {
		t.Helper()
		pending, err := filepath.Glob(filepath.Join(dir, ".bridge-rust-job-*"))
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) != 0 {
			t.Fatalf("%s leaked per-job Rust temporary directories: %v", label, pending)
		}
		pending, err = filepath.Glob(filepath.Join(dir, ".bridge-rust-plan-*.json"))
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) != 0 {
			t.Fatalf("%s leaked Rust plan files: %v", label, pending)
		}
	}

	destination := filepath.Join(dir, "one.3mf")
	result, err := ConvertTarget(source, destination, TargetRequest{Target: "u1"})
	if err != nil {
		t.Fatalf("experimental Rust conversion failed: %v", err)
	}
	if result["engine"] != "rust-3mf" {
		t.Fatalf("incorrect engine: %v", result["engine"])
	}
	if _, err := inspectPackage(destination); err != nil {
		t.Fatal(err)
	}
	noJobs("successful conversion")

	published, err := os.ReadFile(destination)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ConvertTarget(source, destination, TargetRequest{Target: "u1"}); err == nil {
		t.Fatal("repeated conversion overwrote published output")
	}
	afterPublished, err := os.ReadFile(destination)
	if err != nil || !bytes.Equal(published, afterPublished) {
		t.Fatal("duplicate output refusal modified published 3MF")
	}
	noJobs("duplicate output refused")

	// Deliberately feed a bad mapping to the isolated Rust process AFTER
	// Go's valid planning stage. Child failure must not leak its workspace.
	rejectedDestination := filepath.Join(dir, "rejected.3mf")
	plan, err := buildPlan(source, rejectedDestination, func(src NozzleInfo) (*ResolvedTargetProfile, error) {
		return resolveBuiltinTarget("u1", "", "", src.Nozzle)
	})
	if err != nil {
		t.Fatal(err)
	}
	profile, preserved, err := preservedProfile(plan.source, plan.base)
	if err != nil {
		t.Fatal(err)
	}
	for original := range plan.mapping {
		plan.mapping[original] = 64
		break
	}
	stage := filepath.Join(dir, "failed-private.3mf")
	if _, err := convertWithExperimentalRust(plan, profile, preserved, stage); err == nil {
		t.Fatal("Rust child accepted deliberately invalid mapping")
	}
	if _, err := os.Stat(stage); !os.IsNotExist(err) {
		t.Fatalf("failed child published scratch: %v", err)
	}
	noJobs("child refusal")
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("Go/Rust child modified source")
	}
	t.Log("PRIVATE_RUST_JOB_CLEANUP_PASS: success, destination collision, child rejection, immutable source")
}
