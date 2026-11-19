package bridge

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// Self-contained cross-platform RC acceptance: the application passes only a
// synthetic, verified OEM bundle to Rust. Rust derives the source's material
// palette and process settings and generates the complete 3MF archive.
// One, five and mixed-material sources must all agree with the established Go
// output, without installing slicers or touching physical printers.
func TestRustAutonomousGoHostParitySynthetic(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires the exact local freshly compiled Rust binary")
	}
	if !filepath.IsAbs(binary) {
		t.Fatal("Rust probe path must be absolute")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)

	// Synthetic 4-tool Creator 5 bundle: five virtual material slots must
	// remain five even though four physical tools are available.
	profile := syntheticVendorBundleFile(t, "0.4")
	for _, tc := range []struct {
		name string
		slots int
	}{
		{"bambu-one-color.3mf", 1},
		{"bambu-five-colors.3mf", 5},
		{"bambu-mixed-three.3mf", 3},
	} {
		t.Run(tc.name, func(t *testing.T) {
			source := filepath.Join("..", "..", "tests", "fixtures", tc.name)
			original, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			dir := t.TempDir()
			goOutput := filepath.Join(dir, "baseline-go.3mf")
			rustOutput := filepath.Join(dir, "rust-source-owned.3mf")

			requested := TargetRequest{Target: "c5", Nozzle: "0.4", ProfileFile: profile, Engine: "go"}
			goReport, err := ConvertTarget(source, goOutput, requested)
			if err != nil {
				t.Fatalf("Go synthetic OEM baseline refused: %v", err)
			}
			requested.Engine = "rust-native"
			rustReport, err := ConvertTarget(source, rustOutput, requested)
			if err != nil {
				t.Fatalf("Rust autonomous full 3MF conversion refused: %v", err)
			}
			if goReport["engine"] != "bl2std" ||
				rustReport["engine"] != "rust-native-3mf" ||
				rustReport["planner"] != "rust-native" {
				t.Fatalf("Go/Rust engine identity mismatch: Go=%v Rust=%v", goReport, rustReport)
			}
			compareConverted3MFParts(t, goOutput, rustOutput)
			for _, key := range []string{
				"geometry_vertices", "geometry_triangles",
				"target_filament_slots_used", "source_filament_slots",
				"source_to_target_slot_map", "target_nozzle",
				"physical_toolheads", "removed_slice_cache_entries",
				"user_slot_mapping_required", "tool_usage_verification_required",
			} {
				if !reflect.DeepEqual(goReport[key], rustReport[key]) {
					t.Fatalf("Go/Rust receipt mismatch for %s: Go=%v Rust=%v",
						key, goReport[key], rustReport[key])
				}
			}
			if got, ok := rustReport["target_filament_slots_used"].(int); !ok || got != tc.slots {
				t.Fatalf("Rust native silently merged/lost material slots: got %v want %d", rustReport["target_filament_slots_used"], tc.slots)
			}
			if got := rustReport["user_slot_mapping_required"]; got != (tc.slots > 4) {
				t.Fatalf("four physical tools should not imply extra material feeds: got %v for %d slots", got, tc.slots)
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(after, original) {
				t.Fatal("Rust autonomous conversion modified the source")
			}
			originalOutput, err := os.ReadFile(rustOutput)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := ConvertTarget(source, rustOutput, requested); err == nil ||
				ErrorCode(err) != "OUTPUT_EXISTS" {
				t.Fatalf("Rust autonomous output overwrite was not refused: %v", err)
			}
			final, err := os.ReadFile(rustOutput)
			if err != nil || !bytes.Equal(final, originalOutput) {
				t.Fatal("Rust autonomous conversion replaced existing output")
			}
			leftovers, err := filepath.Glob(filepath.Join(dir, ".bridge-*"))
			if err != nil || len(leftovers) != 0 {
				t.Fatalf("Rust autonomous conversion leaked private scratch: %v (%v)", leftovers, err)
			}
		})
	}
}
