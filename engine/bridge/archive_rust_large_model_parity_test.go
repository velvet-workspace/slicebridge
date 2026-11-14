package bridge

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustLargeModelArchiveParity generates 24,000 closed tetrahedra with
// 96,000 real triangles (not an XML comment) and two independently declared
// slice-info plates with stale caches. It verifies >8MiB streamed 3D XML
// and complete 3MF Go/Rust archive parity. This is synthetic QA geometry:
// not real multi-plate production source or permission to slice/print.
func TestRustLargeModelArchiveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "large-two-plate-geometry.3mf")
	const solids = 24000
	createBoundedTwoPlateGeometrySource(t, source, solids)
	archive, err := zip.OpenReader(source)
	if err != nil {
		t.Fatal(err)
	}
	enlarged := false
	for _, entry := range archive.File {
		if entry.Name == "3D/3dmodel.model" {
			if entry.UncompressedSize64 <= 8<<20 || entry.UncompressedSize64 >= 200<<20 {
				t.Fatalf("real geometry did not exercise 8-200 MiB streaming range: %d bytes", entry.UncompressedSize64)
			}
			enlarged = true
			t.Logf("synthetic closed-tetrahedron geometry=%d solids, vertices=%d, triangles=%d, model XML=%d bytes",
				solids, solids*4, solids*4, entry.UncompressedSize64)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	if !enlarged {
		t.Fatal("model entry missing")
	}
	inputBytes, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}

	originalParts := inspectZipEntries(t, source)
	if got := bytes.Count(originalParts["Metadata/slice_info.config"], []byte("<plate>")); got != 2 {
		t.Fatalf("expected two source plate metadata sections, got %d", got)
	}
	for _, stale := range []string{"Metadata/plate_1.gcode", "Metadata/plate_1.gcode.md5", "Metadata/plate_2.gcode", "Metadata/plate_2.gcode.md5", "Metadata/plate_2.json"} {
		if _, ok := originalParts[stale]; !ok {
			t.Fatalf("source no longer has intended stale slice cache %s", stale)
		}
	}
	var sourceSettings map[string]any
	if err := json.Unmarshal(originalParts["Metadata/project_settings.config"], &sourceSettings); err != nil {
		t.Fatal(err)
	}
	insp, err := converter.Inspect(source)
	if err != nil {
		t.Fatalf("Go failed to inspect expanded source: %v", err)
	}
	slots, mapping, err := sourceSlotPlan(insp.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	profile := printer.Builtin("snapmaker-u1")
	if profile == nil {
		t.Fatal("missing builtin test profile")
	}
	rawGo := filepath.Join(dir, "go-raw.3mf")
	goFinal := filepath.Join(dir, "go-final.3mf")
	rustFinal := filepath.Join(dir, "rust-final.3mf")
	if _, err := converter.Convert(source, rawGo, converter.Options{
		Printer: profile, Slots: slots, Mapping: mapping, Supports: converter.SupportsOff,
	}); err != nil {
		t.Fatalf("Go large-model conversion failed: %v", err)
	}
	removed, err := stripSliceCache(rawGo, goFinal, nil)
	if err != nil {
		t.Fatal(err)
	}
	mapped := map[string]int{}
	for src, dst := range mapping {
		mapped[strconv.Itoa(src)] = dst
	}
	plan := map[string]any{
		"profile": profile, "slots": slots, "slot_count": profile.FilamentSlots,
		"mapping": mapped, "supports": false,
		"source_settings": sourceSettings, "preserved": []string{},
	}
	serialized, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	planPath := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(planPath, serialized, 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	output, commandErr := exec.CommandContext(ctx, rust, "--convert-archive", planPath, source, rustFinal).Output()
	if ctx.Err() != nil {
		t.Fatal(ctx.Err())
	}
	var result struct {
		OK      bool `json:"ok"`
		Removed int  `json:"removed"`
	}
	if err := json.Unmarshal(output, &result); err != nil {
		t.Fatalf("Rust reply invalid: %v %s", err, output)
	}
	if commandErr != nil || !result.OK || result.Removed != removed {
		t.Fatalf("Rust failed to convert large valid model: %v %+v", commandErr, result)
	}
	if removed < 6 {
		t.Fatalf("expected independent stale caches for both plates removed: %d", removed)
	}
	compareConverted3MFParts(t, goFinal, rustFinal)
	outputFacts, err := inspectPackage(rustFinal)
	if err != nil {
		t.Fatalf("Go cannot inspect Rust large geometry: %v", err)
	}
	if outputFacts.vertices != solids*4 || outputFacts.triangles != solids*4 || outputFacts.sliceCaches != 0 {
		t.Fatalf("output mesh cardinality/cache changed: vertices=%d triangles=%d cached=%d", outputFacts.vertices, outputFacts.triangles, outputFacts.sliceCaches)
	}
	convertedParts := inspectZipEntries(t, rustFinal)
	if got := bytes.Count(convertedParts["Metadata/slice_info.config"], []byte("<plate")); got != 2 {
		t.Fatalf("two source plates were not preserved after Go/Rust conversion: got %d", got)
	}
	for _, cached := range []string{"Metadata/plate_1.gcode", "Metadata/plate_1.gcode.md5", "Metadata/plate_2.gcode", "Metadata/plate_2.gcode.md5", "Metadata/plate_2.json"} {
		if _, ok := convertedParts[cached]; ok {
			t.Fatalf("stale two-plate cache leaked into published output: %s", cached)
		}
	}
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(inputBytes, after) {
		t.Fatal("source changed during conversion")
	}
	if files, _ := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial")); len(files) != 0 {
		t.Fatalf("streaming archive left scratch partial files: %v", files)
	}
	t.Logf("Go/Rust full 3MF ZIP parity on 24,000 tetrahedra, 96,000 triangles and 2 distinct plate metadata sections; compressed source=%d bytes; no stale caches", len(inputBytes))
}
