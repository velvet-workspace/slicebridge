package bridge

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// One physical QIDI X-Plus 5 print tool remains one even if its installed
// slicer has multiple *virtual* project palette slots. A QIDI BOX feeder
// (which is not present on the owner's printer) must not be confused with
// multiple physical nozzles. No user files, slicers or printers are touched.
func TestQIDIOnePhysicalToolKeepsSourceFilamentsVirtual(t *testing.T) {
	for _, sourceCount := range []int{1, 5} {
		t.Run(fmt.Sprintf("source-%d", sourceCount), func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			target := filepath.Join(dir, "qidi.3mf")
			fixture(t, source, sourceCount)
			setSourceNozzle(t, source, "0.4")
			before, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			digest := sha256.Sum256(before)

			// The 4-element machine nozzle array deliberately mirrors
			// installed vendor metadata. It cannot be interpreted as four
			// independent physical print tools on X-Plus 5.
			bundle := syntheticMaterialBundle()
			bundle.Target = "qidi"
			bundle.Nozzle = "0.4"
			bundle.Profile.Name = "qidi"
			bundle.Profile.DisplayName = "X-Plus 5"
			bundle.Profile.ModelID = "X-Plus 5"
			bundle.Profile.ProjectSettings["printer_model"] = "X-Plus 5"
			encoded, err := json.Marshal(bundle)
			if err != nil {
				t.Fatal(err)
			}
			bundlePath := filepath.Join(dir, "qidi-installed-synthetic-bundle.json")
			if err := os.WriteFile(bundlePath, encoded, 0600); err != nil {
				t.Fatal(err)
			}
			result, err := ConvertTarget(source, target, TargetRequest{
				Target: "qidi", ProfileFile: bundlePath, Nozzle: "0.4",
			})
			if err != nil {
				t.Fatalf("single tool virtual palette conversion: %v (%s)", err, ErrorCode(err))
			}
			for key, want := range map[string]int{
				"physical_toolheads":         1,
				"source_filament_slots":      sourceCount,
				"virtual_filament_slots":     sourceCount,
				"target_filament_slots_used": sourceCount,
			} {
				if got := result[key]; got != want {
					t.Fatalf("incorrect QIDI tool/palette report %s=%v (want %d)", key, got, want)
				}
			}
			if got := result["user_slot_mapping_required"]; got != (sourceCount > 1) {
				t.Fatalf("single tool must require a material/slot review for a multi-slot source: %v", got)
			}
			if got := result["tool_usage_verification_required"]; got != (sourceCount > 1) {
				t.Fatalf("multicolor native tool-use review missing: %v", got)
			}
			slotMap, ok := result["source_to_target_slot_map"].(map[int]int)
			if !ok || len(slotMap) != sourceCount {
				t.Fatalf("virtual slots were collapsed: %+v", result)
			}
			for i := 1; i <= sourceCount; i++ {
				if slotMap[i] != i {
					t.Fatalf("source material %d was silently folded: %v", i, slotMap)
				}
			}
			slicerSettings := projectSettings(t, target)
			if got := slicerSettings["printer_model"]; got != "X-Plus 5" {
				t.Fatalf("QIDI printer identity changed: %v", got)
			}
			if nozzle, ok := slicerSettings["nozzle_diameter"].([]any); !ok || len(nozzle) != 4 {
				t.Fatalf("test fixture's manufacturer nozzle metadata was overwritten: %v", slicerSettings["nozzle_diameter"])
			}
			colors, ok := slicerSettings["filament_colour"].([]any)
			if !ok || len(colors) < sourceCount {
				t.Fatalf("conversion lost source virtual palette: %v", slicerSettings["filament_colour"])
			}
			for i := 0; i < sourceCount; i++ {
				if colors[i] != fixtureColors[i] {
					t.Fatalf("source color slot %d changed: %v", i+1, colors)
				}
			}
			warnings := strings.Join(result["warnings"].([]string), "\n")
			hasNoBoxWarning := strings.Contains(warnings, "Without a verified QIDI BOX")
			if hasNoBoxWarning != (sourceCount > 1) {
				t.Fatalf("single vs multi QIDI hardware warning mismatch: %s", warnings)
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(before, after) || sha256.Sum256(after) != digest {
				t.Fatal("conversion mutated original 3MF")
			}
		})
	}
}
