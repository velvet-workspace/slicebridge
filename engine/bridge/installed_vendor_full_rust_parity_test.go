package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustFullArchiveInstalledVendorProfileParity requires an explicit,
// user-authorized, locally flattened snapshot from the real native preset
// resolver. It never searches user profiles, starts slicers or sends printer
// commands. This checks the independent Rust ZIP transformer against the Go
// converter with the *same real OEM machine/process/material baseline*.
// Native material binding in the Go bridge is checked separately and is NOT
// recreated or asserted by this lower-level converter comparison.
func TestRustFullArchiveInstalledVendorProfileParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if rust == "" || root == "" {
		t.Skip("set Rust executable and approved local installed-vendor snapshot directory")
	}
	cases := []struct {
		target string
		source string
	}{
		{"u1", "bambu-one-color.3mf"},
		{"u1", "bambu-five-colors.3mf"},
		{"qidi", "bambu-one-color.3mf"},
		{"qidi", "bambu-five-colors.3mf"},
	}
	for _, tc := range cases {
		t.Run(tc.target+"_"+tc.source, func(t *testing.T) {
			rawBundle, err := os.ReadFile(filepath.Join(root, fmt.Sprintf("scratch-%s-0.4-resolved.json", tc.target)))
			if err != nil {
				t.Fatal(err)
			}
			if len(rawBundle) < 100 || len(rawBundle) > 4<<20 {
				t.Fatal("installed-profile snapshot outside 100 B .. 4 MiB limit")
			}
			var bundle struct {
				Target  string          `json:"target"`
				Nozzle  string          `json:"nozzle"`
				Profile printer.Profile `json:"profile"`
			}
			if err := json.Unmarshal(rawBundle, &bundle); err != nil {
				t.Fatal(err)
			}
			if bundle.Target != tc.target || bundle.Nozzle != "0.4" ||
				bundle.Profile.ModelID == "" || len(bundle.Profile.ProjectSettings) < 40 ||
				bundle.Profile.FilamentSlots != 4 {
				t.Fatalf("installed vendor snapshot identity invalid, target=%q nozzle=%q model=%q keys=%d slots=%d",
					bundle.Target, bundle.Nozzle, bundle.Profile.ModelID, len(bundle.Profile.ProjectSettings), bundle.Profile.FilamentSlots)
			}
			for _, key := range []string{
				"machine_start_gcode", "machine_end_gcode", "change_filament_gcode",
				"printer_model", "printer_settings_id", "print_settings_id",
			} {
				if _, ok := bundle.Profile.ProjectSettings[key]; !ok {
					t.Fatalf("installed OEM snapshot lacks critical setting %s", key)
				}
			}
			src := filepath.Join("..", "..", "tests", "fixtures", tc.source)
			before, err := os.ReadFile(src)
			if err != nil {
				t.Fatal(err)
			}
			insp, err := converter.Inspect(src)
			if err != nil {
				t.Fatal(err)
			}
			slots, mapping, err := sourceSlotPlan(insp.Filaments)
			if err != nil {
				t.Fatal(err)
			}
			if len(slots) > bundle.Profile.FilamentSlots {
				bundle.Profile.FilamentSlots = len(slots)
			}
			if err := bundle.Profile.Validate(); err != nil {
				t.Fatalf("installed vendor profile is invalid: %v", err)
			}
			var sourceSettings map[string]any
			if err := json.Unmarshal(inspectZipEntries(t, src)["Metadata/project_settings.config"], &sourceSettings); err != nil {
				t.Fatal(err)
			}
			dir := t.TempDir()
			goRaw := filepath.Join(dir, "upstream.3mf")
			goFinal := filepath.Join(dir, "go-final.3mf")
			rustFinal := filepath.Join(dir, "rust-final.3mf")
			_, err = converter.Convert(src, goRaw, converter.Options{
				Printer: &bundle.Profile, Slots: slots, Mapping: mapping,
				Supports: converter.SupportsOff,
			})
			if err != nil {
				t.Fatalf("Go OEM-base conversion failed: %v", err)
			}
			removedGo, err := stripSliceCache(goRaw, goFinal, nil)
			if err != nil {
				t.Fatal(err)
			}
			jsonMapping := make(map[string]int, len(mapping))
			for old, current := range mapping {
				jsonMapping[strconv.Itoa(old)] = current
			}
			plan := map[string]any{
				"profile":         bundle.Profile,
				"slots":           slots,
				"slot_count":      bundle.Profile.FilamentSlots,
				"supports":        false,
				"source_settings": sourceSettings,
				"mapping":         jsonMapping,
				"preserved":       []string{},
			}
			data, err := json.Marshal(plan)
			if err != nil {
				t.Fatal(err)
			}
			planFile := filepath.Join(dir, "explicit-oem-plan.json")
			if err := os.WriteFile(planFile, data, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
			defer cancel()
			output, commandErr := exec.CommandContext(ctx, rust,
				"--convert-archive", planFile, src, rustFinal).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var result struct {
				OK      bool `json:"ok"`
				Removed int  `json:"removed"`
			}
			if err := json.Unmarshal(output, &result); err != nil {
				t.Fatalf("Rust returned malformed JSON: %v, command=%v, output=%s", err, commandErr, output)
			}
			if commandErr != nil || !result.OK || result.Removed != removedGo {
				t.Fatalf("Rust rejected Go-valid installed OEM project: command=%v result=%+v Go cache removal=%d", commandErr, result, removedGo)
			}
			compareConverted3MFParts(t, goFinal, rustFinal)
			var settings map[string]any
			if err := json.Unmarshal(inspectZipEntries(t, rustFinal)["Metadata/project_settings.config"], &settings); err != nil {
				t.Fatal(err)
			}
			for _, key := range []string{
				"machine_start_gcode", "machine_end_gcode", "change_filament_gcode",
				"printer_model", "printer_settings_id", "print_settings_id",
			} {
				if fmt.Sprint(settings[key]) != fmt.Sprint(bundle.Profile.ProjectSettings[key]) {
					t.Fatalf("Rust changed installed OEM machine-owned setting %s", key)
				}
			}
			after, err := os.ReadFile(src)
			if err != nil || !bytes.Equal(before, after) {
				t.Fatal("source 3MF was modified by installed vendor parity check")
			}
			// Persist only explicitly requested, isolated QA artifacts.
			// The exclusive-create helper refuses to overwrite any existing
			// file and never writes into installed slicer/profile folders.
			if outputDir := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR"); outputDir != "" {
				if st, err := os.Stat(outputDir); err != nil || !st.IsDir() {
					t.Fatalf("explicit native OEM QA output dir missing: %s: %v", outputDir, err)
				}
				for _, item := range [][2]string{
					{src, "oem-source-" + tc.target + "-" + tc.source},
					{goFinal, "oem-go-" + tc.target + "-" + tc.source},
					{rustFinal, "oem-rust-" + tc.target + "-" + tc.source},
				} {
					exportSlicerFixtureNoOverwrite(t, item[0], filepath.Join(outputDir, item[1]))
				}
			}
			t.Logf("OEM machine=%s, virtual slots=%d, source materials=%d: Go and Rust full 3MF ZIP parity passed",
				bundle.Profile.ModelID, bundle.Profile.FilamentSlots, len(slots))
		})
	}
}
