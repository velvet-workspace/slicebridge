package bridge

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// The exact SupportsAuto predicate is part of the Go/Rust plan contract.
// A source override should not silently change supports when moving engines.
func TestRustSourceSupportPlanMatchesUpstreamAuto(t *testing.T) {
	for _, tc := range []struct {
		name string
		data map[string]any
		want bool
	}{
		{"empty", map[string]any{}, false},
		{"explicit-one", map[string]any{"enable_support": "1"}, true},
		{"explicit-zero", map[string]any{"enable_support": "0"}, false},
		{"numeric-one-is-not-source-auto", map[string]any{"enable_support": 1}, false},
		{"override", map[string]any{"different_settings_to_system": []any{"layer_height", "enable_support"}}, true},
		{"near-key", map[string]any{"different_settings_to_system": []any{"support_threshold_angle"}}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := sourceSupportsForRust(tc.data); got != tc.want {
				t.Fatalf("supports Auto parity: got %t want %t", got, tc.want)
			}
		})
	}
}

// This opt-in test exercises the actual public production ConvertTarget
// entrypoint, never Rust test-only reconstruction of converted Go settings.
// The *only* difference is the explicit engine choice. Compare every ZIP
// member, all machine/material settings, and the source file's immutable bytes.
func TestRustSelectedProductionConversionParity(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires approved local Rust release binary in SLICEBRIDGE_RUST_PROBE")
	}
	oem := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	cases := []struct {
		target, source string
		vendor         bool
	}{
		{"u1", "bambu-one-color.3mf", false},
		{"u1", "bambu-five-colors.3mf", false},
		{"qidi", "bambu-one-color.3mf", true},
		{"qidi", "bambu-five-colors.3mf", true},
		{"u1", "bambu-five-colors.3mf", true},
	}
	for _, tc := range cases {
		t.Run(tc.target+"_"+tc.source+"_"+map[bool]string{false: "builtin", true: "oem"}[tc.vendor], func(t *testing.T) {
			if tc.vendor && oem == "" {
				t.Skip("approved installed read-only OEM snapshots unavailable")
			}
			dir := t.TempDir()
			b, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", tc.source))
			if err != nil {
				t.Fatal(err)
			}
			source := filepath.Join(dir, tc.source)
			if err := os.WriteFile(source, b, 0600); err != nil {
				t.Fatal(err)
			}
			req := TargetRequest{Target: tc.target}
			if tc.vendor {
				req.ProfileFile = filepath.Join(oem, "scratch-"+tc.target+"-0.4-resolved.json")
				req.Nozzle = "0.4"
			}
			goOutput := filepath.Join(dir, "baseline-go.3mf")
			rustOutput := filepath.Join(dir, "pilot-rust.3mf")
			t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "")
			t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
			goResult, err := ConvertTarget(source, goOutput, req)
			if err != nil {
				t.Fatalf("actual Go baseline conversion refused: %v", err)
			}
			t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust")
			rustResult, err := ConvertTarget(source, rustOutput, req)
			if err != nil {
				t.Fatalf("actual production entrypoint with Rust selected refused: %v", err)
			}
			if goResult["engine"] != "bl2std" || rustResult["engine"] != "rust-3mf" {
				t.Fatalf("wrong engine selected: Go=%v Rust=%v", goResult["engine"], rustResult["engine"])
			}
			compareConverted3MFParts(t, goOutput, rustOutput)
			for _, key := range []string{
				"geometry_vertices", "geometry_triangles", "removed_slice_cache_entries",
				"target_filament_slots_used", "source_filament_slots", "target_nozzle",
				"prime_tower_plates", "prime_tower_plate_position_audit_status",
				"target_nozzle_origin", "profile_kind", "virtual_filament_slots",
				"physical_toolheads", "user_slot_mapping_required",
			} {
				if !reflect.DeepEqual(goResult[key], rustResult[key]) {
					t.Fatalf("%s differed from Go baseline: got Rust=%v Go=%v", key, rustResult[key], goResult[key])
				}
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(b, after) {
				t.Fatal("production Rust path mutated original 3MF")
			}
			// Both engines must create exclusively. A second identical output
			// request must fail with no mutation of the first completed file.
			stable, err := os.ReadFile(rustOutput)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := ConvertTarget(source, rustOutput, req); err == nil {
				t.Fatal("repeated Rust output unexpectedly overwrote a completed project")
			}
			afterFailed, err := os.ReadFile(rustOutput)
			if err != nil || !bytes.Equal(stable, afterFailed) {
				t.Fatal("failed exclusive publication altered previous Rust output")
			}
			t.Logf("ACTUAL_PUBLIC_ENTRYPOINT_GO_RUST_PARITY: %s/%s, target OEM=%t", tc.target, tc.source, tc.vendor)
		})
	}
}

// Explicit Rust selection must never fall back silently to Go, overwrite
// existing output or require the user to repair an interrupted job manually.
func TestRustSelectedEngineMissingBinaryFailsClosed(t *testing.T) {
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", "")
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	original := fixture(t, source, 1)
	destination := filepath.Join(dir, "rust.3mf")
	_, err := ConvertTarget(source, destination, TargetRequest{Target: "u1"})
	if err == nil || ErrorCode(err) != "RUST_ENGINE_FAILED" ||
		!strings.Contains(err.Error(), "SLICEBRIDGE_RUST_ENGINE_BIN") {
		t.Fatalf("unavailable Rust engine must fail closed with a useful reason: %v", err)
	}
	if _, err := os.Stat(destination); !os.IsNotExist(err) {
		t.Fatal("missing Rust binary unexpectedly created output")
	}
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(original, after) {
		t.Fatal("missing Rust binary altered source")
	}
	matches, err := filepath.Glob(filepath.Join(dir, ".bridge-*"))
	if err != nil || len(matches) != 0 {
		t.Fatalf("missing Rust binary left scratch artifacts: %v, %v", matches, err)
	}
}

func TestUnknownRustEngineConfigurationFailsBeforeCreatingOutput(t *testing.T) {
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "roost")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", "")
	dir := t.TempDir()
	src := filepath.Join(dir, "source.3mf")
	before := fixture(t, src, 1)
	output := filepath.Join(dir, "unexpected.3mf")
	_, err := ConvertTarget(src, output, TargetRequest{Target: "u1"})
	if err == nil || ErrorCode(err) != "ENGINE_INVALID" {
		t.Fatalf("unknown engine selector must fail closed: %v", err)
	}
	if _, err := os.Stat(output); !os.IsNotExist(err) {
		t.Fatal("invalid engine selection published an output")
	}
	raw, err := os.ReadFile(src)
	if err != nil || !bytes.Equal(raw, before) {
		t.Fatal("invalid engine selector changed source bytes")
	}
}

func TestRustEngineDefaultDoesNotDependOnConfiguredBinary(t *testing.T) {
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", filepath.Join(t.TempDir(), "nonexistent-rust-cli"))
	dir := t.TempDir()
	src := filepath.Join(dir, "source.3mf")
	fixture(t, src, 1)
	output := filepath.Join(dir, "go.3mf")
	got, err := ConvertTarget(src, output, TargetRequest{Target: "u1"})
	if err != nil || got["engine"] != "bl2std" {
		t.Fatalf("installed production default must remain Go: %v result=%v", err, got)
	}
	if _, err := os.Stat(output); err != nil {
		t.Fatalf("default Go output missing: %v", err)
	}
}

func TestFrozenRustInputAnchorsGoPlanToImmutableSource(t *testing.T) {
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	original := fixture(t, source, 5)
	frozen, cleanup, err := freezeRustSourceForPlan(source, filepath.Join(dir, "final.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	defer cleanup()
	if filepath.Base(frozen) == filepath.Base(source) {
		t.Fatal("snapshot must never alias original source")
	}
	inside, err := os.ReadFile(frozen)
	if err != nil || !bytes.Equal(inside, original) {
		t.Fatal("frozen source does not match the original archive")
	}
	if err := os.WriteFile(source, bytes.Repeat([]byte("x"), len(original)), 0600); err != nil {
		t.Fatal(err)
	}
	stillFrozen, err := os.ReadFile(frozen)
	if err != nil || !bytes.Equal(stillFrozen, original) {
		t.Fatal("source replaced after capture affected Go/Rust plan snapshot")
	}
	cleanup()
	if _, err := os.Stat(frozen); !os.IsNotExist(err) {
		t.Fatal("source snapshot cleanup failed")
	}
}
func TestFrozenRustInputRejectsInvalidSourceAndLeavesNoScratch(t *testing.T) {
	dir := t.TempDir()
	empty := filepath.Join(dir, "empty.3mf")
	if err := os.WriteFile(empty, nil, 0600); err != nil {
		t.Fatal(err)
	}
	dest := filepath.Join(dir, "output.3mf")
	if _, cleanup, err := freezeRustSourceForPlan(empty, dest); err == nil || cleanup != nil {
		t.Fatalf("empty 3MF snapshot must fail safely: %v", err)
	}
	if _, cleanup, err := freezeRustSourceForPlan(filepath.Join(dir, "missing.3mf"), dest); err == nil || cleanup != nil {
		t.Fatalf("missing 3MF snapshot must fail safely: %v", err)
	}
	if _, err := os.Stat(dest); !os.IsNotExist(err) {
		t.Fatal("invalid source created a destination")
	}
	scratch, err := filepath.Glob(filepath.Join(dir, ".bridge-rust-source-*"))
	if err != nil || len(scratch) != 0 {
		t.Fatalf("invalid input leaked Rust source snapshots: %v %v", scratch, err)
	}
}

func TestRustChildTranscriptMemoryBoundAndDrain(t *testing.T) {
	var out cappedRustTranscript
	out.limit = 64 << 10
	chunk := bytes.Repeat([]byte("D"), 200<<10)
	n, err := out.Write(chunk)
	if err != nil || n != len(chunk) {
		t.Fatalf("oversized child stdout must be drained without short writes: count=%d err=%v", n, err)
	}
	if !out.truncated || out.buffer.Len() != 64<<10 {
		t.Fatalf("oversized output was not bounded: truncated=%t size=%d", out.truncated, out.buffer.Len())
	}
	n, err = out.Write(chunk)
	if err != nil || n != len(chunk) || out.buffer.Len() != 64<<10 {
		t.Fatal("subsequent child stdout was not drained without memory growth")
	}
	out = cappedRustTranscript{limit: 64 << 10}
	ok := []byte("{\"ok\":true,\"experimental\":true,\"removed\":1}\n")
	n, err = out.Write(ok)
	if err != nil || n != len(ok) || out.truncated || !bytes.Equal(out.buffer.Bytes(), ok) {
		t.Fatalf("small valid Rust receipt was damaged: %+v err=%v", out, err)
	}
}
