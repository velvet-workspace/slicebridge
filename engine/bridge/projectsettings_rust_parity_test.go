package bridge

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustProjectSettingsConverted3MFParity compares the isolated Rust
// project_settings composer against the project_settings.config inside
// *actual* Go-converted 3MF archives, including explicit support choices,
// printer-profile overrides and nonstandard hardware slot counts.
// No installed slicer, printer or production application is contacted.
func TestRustProjectSettingsConverted3MFParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe binary")
	}

	for _, slotCount := range []int{1, 4, 6} {
		for _, enabled := range []bool{false, true} {
			for _, override := range []bool{false, true} {
				name := fmt.Sprintf("%d_slots_support_%t_override_%t", slotCount, enabled, override)
				t.Run(name, func(t *testing.T) {
					dir := t.TempDir()
					source := filepath.Join(dir, "input.3mf")
					target := filepath.Join(dir, "converted-go.3mf")
					fixture(t, source, slotCount)

					// Add user support options to a source fixture without
					// modifying any original/customer 3MF.
					parts := inspectZipEntries(t, source)
					var original map[string]any
					if err := json.Unmarshal(parts["Metadata/project_settings.config"], &original); err != nil {
						t.Fatal(err)
					}
					original["support_type"] = "tree"
					original["support_threshold_angle"] = "35"
					original["support_style"] = "organic"
					original["support_on_build_plate_only"] = "1"
					updated, err := json.Marshal(original)
					if err != nil {
						t.Fatal(err)
					}
					replaceZIPPart(t, source, "Metadata/project_settings.config", updated)

					inspection, err := converter.Inspect(source)
					if err != nil {
						t.Fatal(err)
					}
					slots, mapping, err := sourceSlotPlan(inspection.Filaments)
					if err != nil {
						t.Fatal(err)
					}
					if len(slots) != slotCount {
						t.Fatalf("expected %d source slots, got %d", slotCount, len(slots))
					}
					profile := printer.Builtin("snapmaker-u1")
					if profile == nil {
						t.Fatal("Go builtin profile not installed in dependency")
					}
					if slotCount > profile.FilamentSlots {
						profile.FilamentSlots = slotCount
					}
					if override {
						slots[0].Profile = "Explicit custom test material profile"
					}
					supportMode := converter.SupportsOff
					if enabled {
						supportMode = converter.SupportsOn
					}
					before, err := json.Marshal(profile.ProjectSettings)
					if err != nil {
						t.Fatal(err)
					}

					// Project metadata from this real conversion is the
					// oracle, not an independently re-created expected map.
					_, err = converter.Convert(source, target, converter.Options{
						Printer: profile, Slots: slots, Mapping: mapping,
						Supports: supportMode,
					})
					if err != nil {
						t.Fatalf("Go fixture conversion failed: %v", err)
					}

					reply, rustErr := runRustMaterialCommand(t, rust, "--compose-project-settings", map[string]any{
						"profile":         profile,
						"slots":           slots,
						"slot_count":      profile.FilamentSlots,
						"supports":        enabled,
						"source_settings": original,
					})
					if rustErr != nil || reply["ok"] != true {
						t.Fatalf("Rust settings composer rejected Go-valid scenario: %v %v", rustErr, reply)
					}
					output := inspectZipEntries(t, target)
					var want map[string]any
					if err := json.Unmarshal(output["Metadata/project_settings.config"], &want); err != nil {
						t.Fatalf("Go-produced project settings invalid: %v", err)
					}
					parityJSON(t, want, reply["project_settings"])

					after, err := json.Marshal(profile.ProjectSettings)
					if err != nil || !bytes.Equal(before, after) {
						t.Fatal("target profile baseline was unexpectedly mutated")
					}
					if enabled {
						if want["enable_support"] != "1" {
							t.Fatal("supports not enabled by Go")
						}
					} else if want["enable_support"] != "0" {
						t.Fatal("supports not disabled by Go")
					}
				})
			}
		}
	}
}
