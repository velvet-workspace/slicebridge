package bridge

import (
    "bytes"
    "os"
    "path/filepath"
    "reflect"
    "testing"
)

// A self-contained CI acceptance: the application passes only a verified
// installed target bundle to Rust. Rust derives every source filament,
// color and process setting directly from the frozen 3MF and generates the
// complete archive. Go independently validates and publishes the result.
// Nothing touches installed profiles, slicers or physical printers.
func TestRustAutonomousGoHostParitySynthetic(t *testing.T) {
    binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
    if binary == "" {
        t.Skip("requires the exact local freshly compiled Rust binary")
    }
    if !filepath.IsAbs(binary) {
        t.Fatal("Rust probe path must be absolute")
    }
    t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)

    // The synthetic installed Creator 5 bundle has a real 4-tool profile
    // shape and intentionally different PLA/PETG material settings.
    profile := syntheticVendorBundleFile(t, "0.4")
    source := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
    original, err := os.ReadFile(source)
    if err != nil {
        t.Fatal(err)
    }
    dir := t.TempDir()
    goOutput := filepath.Join(dir, "baseline-go.3mf")
    rustOutput := filepath.Join(dir, "rust-source-owned.3mf")

    requested := TargetRequest{Target:"c5", Nozzle:"0.4", ProfileFile:profile, Engine:"go"}
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
        "geometry_vertices", "geometry_triangles", "target_filament_slots_used",
        "source_filament_slots", "source_to_target_slot_map", "target_nozzle",
        "physical_toolheads", "removed_slice_cache_entries",
    } {
        if !reflect.DeepEqual(goReport[key], rustReport[key]) {
            t.Fatalf("Go/Rust receipt mismatch for %s: Go=%v Rust=%v",
                key, goReport[key], rustReport[key])
        }
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
}
