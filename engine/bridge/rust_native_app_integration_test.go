package bridge

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// Tests the REAL public conversion entrypoint. With rust-native selected no
// Go conversionPlan, Go slot map or Go material binding reaches Rust.
func TestRustNativeSelectedProductionOEMParity(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	oemRoot := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if binary == "" || oemRoot == "" {
		t.Skip("requires approved Rust binary and read-only installed OEM snapshots")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	for _, target := range []string{"u1", "qidi"} {
		for _, fixtureName := range []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"} {
			t.Run(target+"_"+fixtureName, func(t *testing.T) {
				dir := t.TempDir()
				input, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", fixtureName))
				if err != nil {
					t.Fatal(err)
				}
				source := filepath.Join(dir, "source.3mf")
				if err := os.WriteFile(source, input, 0600); err != nil {
					t.Fatal(err)
				}
				req := TargetRequest{
					Target: target, Nozzle: "0.4",
					ProfileFile: filepath.Join(oemRoot, "scratch-"+target+"-0.4-resolved.json"),
				}
				t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "go")
				goOut := filepath.Join(dir, "baseline.3mf")
				goReceipt, err := ConvertTarget(source, goOut, req)
				if err != nil {
					t.Fatalf("Go baseline: %v", err)
				}
				t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust-native")
				rustOut := filepath.Join(dir, "native.3mf")
				native, err := ConvertTarget(source, rustOut, req)
				if err != nil {
					t.Fatalf("app Rust-native route: %v", err)
				}
				if goReceipt["engine"] != "bl2std" ||
					native["engine"] != "rust-native-3mf" ||
					native["planner"] != "rust-native" {
					t.Fatalf("wrong engine or planner: Go=%v Rust=%v", goReceipt, native)
				}
				compareConverted3MFParts(t, goOut, rustOut)
				for _, key := range []string{
					"geometry_vertices", "geometry_triangles",
					"target_nozzle", "target_nozzle_origin", "profile_kind",
					"target_filament_slots_used", "source_filament_slots",
					"source_to_target_slot_map", "physical_toolheads",
					"removed_slice_cache_entries", "prime_tower_plates",
					"prime_tower_plate_position_audit_status",
				} {
					if !reflect.DeepEqual(goReceipt[key], native[key]) {
						t.Fatalf("%s: Go=%v native=%v", key, goReceipt[key], native[key])
					}
				}
				unchanged, err := os.ReadFile(source)
				if err != nil || !bytes.Equal(unchanged, input) {
					t.Fatal("Rust-native modified original source")
				}
				published, err := os.ReadFile(rustOut)
				if err != nil {
					t.Fatal(err)
				}
				if _, err := ConvertTarget(source, rustOut, req); err == nil || ErrorCode(err) != "OUTPUT_EXISTS" {
					t.Fatalf("Rust-native did not refuse existing output: %v", err)
				}
				same, err := os.ReadFile(rustOut)
				if err != nil || !bytes.Equal(same, published) {
					t.Fatal("Rust-native overwritten published output")
				}
				leftovers, err := filepath.Glob(filepath.Join(dir, ".bridge-*"))
				if err != nil || len(leftovers) != 0 {
					t.Fatalf("private scratch files left behind: %v (%v)", leftovers, err)
				}
			})
		}
	}
}

func TestRustNativeCancelledContextLeavesNoOutput(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	oemRoot := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if binary == "" || oemRoot == "" {
		t.Skip("requires approved Rust executable and OEM snapshots")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	original := fixture(t, source, 1)
	dst := filepath.Join(dir, "cancelled.3mf")
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust-native")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err := ConvertTargetContext(ctx, source, dst, TargetRequest{
		Target: "u1", Nozzle: "0.4",
		ProfileFile: filepath.Join(oemRoot, "scratch-u1-0.4-resolved.json"),
	})
	if err == nil || ErrorCode(err) != "RUST_ENGINE_FAILED" {
		t.Fatalf("canceled Rust-native should fail closed: %v", err)
	}
	if _, err := os.Stat(dst); !os.IsNotExist(err) {
		t.Fatal("cancel produced an output")
	}
	raw, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(original, raw) {
		t.Fatal("cancel modified source")
	}
	leftovers, err := filepath.Glob(filepath.Join(dir, ".bridge-*"))
	if err != nil || len(leftovers) != 0 {
		t.Fatalf("cancel leaked private scratch files: %v (%v)", leftovers, err)
	}
}

func TestRustNativeRejectsBuiltinWithoutGuessingOEM(t *testing.T) {
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust-native")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", filepath.Join(t.TempDir(), "missing.exe"))
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	fixture(t, source, 1)
	dst := filepath.Join(dir, "unsupported.3mf")
	_, err := ConvertTarget(source, dst, TargetRequest{Target: "u1"})
	if err == nil || ErrorCode(err) != "TARGET_PROFILE_REQUIRED" {
		t.Fatalf("Rust-native must not invent a vendor OEM profile: %v", err)
	}
	if _, err := os.Stat(dst); !os.IsNotExist(err) {
		t.Fatal("unsupported native plan created output")
	}
}
