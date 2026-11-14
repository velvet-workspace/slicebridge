package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustFullArchiveExperimentalParity is an opt-in, scratch-only
// end-to-end differential test. It verifies the experimental Rust 3MF ZIP
// assembler against an actual Go converter output + Go cache sanitizer,
// not against a hand-authored expected output. No real machine is contacted.
func TestRustFullArchiveExperimentalParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to compiled Rust binary")
	}
	cases := []struct {
		source, target int
		supports       bool
	}{
		{1, 4, false},
		{4, 4, false},
		{4, 4, true},
		{6, 6, false},
		{6, 4, false}, // explicit many-to-one mapping
		{6, 4, true},
	}
	for _, tc := range cases {
		t.Run(fmt.Sprintf("%d_to_%d_supports_%t", tc.source, tc.target, tc.supports), func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			rawGo := filepath.Join(dir, "go-upstream.3mf")
			goFinal := filepath.Join(dir, "go-final.3mf")
			rustFinal := filepath.Join(dir, "rust-final.3mf")
			fixture(t, source, tc.source)
			sourceBytes, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			sourceParts := inspectZipEntries(t, source)
			sourceSettings := map[string]any{}
			if err := json.Unmarshal(sourceParts["Metadata/project_settings.config"], &sourceSettings); err != nil {
				t.Fatal(err)
			}
			insp, err := converter.Inspect(source)
			if err != nil {
				t.Fatal(err)
			}
			slots, mapping, err := sourceSlotPlan(insp.Filaments)
			if err != nil {
				t.Fatal(err)
			}
			if tc.target < len(slots) {
				slots = slots[:tc.target]
				for original := range mapping {
					if mapping[original] > tc.target {
						mapping[original] = (original-1)%tc.target + 1
					}
				}
			}
			profile := printer.Builtin("snapmaker-u1")
			if profile == nil {
				t.Fatal("missing builtin Go test printer")
			}
			profile.FilamentSlots = tc.target
			mode := converter.SupportsOff
			if tc.supports {
				mode = converter.SupportsOn
			}
			_, err = converter.Convert(source, rawGo, converter.Options{
				Printer: profile, Slots: slots, Mapping: mapping, Supports: mode,
			})
			if err != nil {
				t.Fatalf("Go 3MF conversion failed: %v", err)
			}
			preserved := []string{"layer_height", "enable_support"}
			goRemoved, err := stripSliceCache(rawGo, goFinal, preserved)
			if err != nil {
				t.Fatalf("Go cache stripping failed: %v", err)
			}
			mappingJSON := map[string]int{}
			for original, destination := range mapping {
				mappingJSON[strconv.Itoa(original)] = destination
			}
			plan := map[string]any{
				"profile":         profile,
				"slots":           slots,
				"slot_count":      tc.target,
				"supports":        tc.supports,
				"source_settings": sourceSettings,
				"mapping":         mappingJSON,
				"preserved":       preserved,
			}
			encoded, err := json.Marshal(plan)
			if err != nil {
				t.Fatal(err)
			}
			planFile := filepath.Join(dir, "explicit-plan.json")
			if err := os.WriteFile(planFile, encoded, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
			defer cancel()
			stdout, runErr := exec.CommandContext(ctx, rust,
				"--convert-archive", planFile, source, rustFinal).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var reply struct {
				OK           bool   `json:"ok"`
				Removed      int    `json:"removed"`
				Experimental bool   `json:"experimental"`
				Error        string `json:"error"`
			}
			if err := json.Unmarshal(stdout, &reply); err != nil {
				t.Fatalf("Rust CLI returned invalid JSON: %v: %s", err, stdout)
			}
			if runErr != nil || !reply.OK || !reply.Experimental || reply.Removed != goRemoved {
				t.Fatalf("Rust conversion failed or count differs: %v %+v Go removed %d", runErr, reply, goRemoved)
			}
			compareConverted3MFParts(t, goFinal, rustFinal)

			facts, err := inspectPackage(rustFinal)
			if err != nil {
				t.Fatalf("Go cannot inspect Rust output: %v", err)
			}
			if facts.sliceCaches != 0 {
				t.Fatalf("Rust left %d stale slice caches", facts.sliceCaches)
			}
			if facts.printerModel != profile.ModelID {
				t.Fatalf("Rust converted model label %q != %q", facts.printerModel, profile.ModelID)
			}
			unchanged, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(sourceBytes, unchanged) {
				t.Fatal("Rust changed the source archive")
			}
			// A second attempt may not overwrite the completed destination.
			beforeDest, err := os.ReadFile(rustFinal)
			if err != nil {
				t.Fatal(err)
			}
			ctx2, cancel2 := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel2()
			rejected, repeatErr := exec.CommandContext(ctx2, rust,
				"--convert-archive", planFile, source, rustFinal).Output()
			if repeatErr == nil {
				t.Fatalf("Rust overwrote an existing output: %s", rejected)
			}
			afterDest, err := os.ReadFile(rustFinal)
			if err != nil || !bytes.Equal(beforeDest, afterDest) {
				t.Fatal("existing converted output mutated")
			}
			// Explicitly opt in to durable *scratch-only* artifacts for
			// inspection by a native slicer. Never overwrite a prior result.
			if outputDir := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR"); outputDir != "" {
				if stat, err := os.Stat(outputDir); err != nil || !stat.IsDir() {
					t.Fatalf("slicer output directory does not exist: %q: %v", outputDir, err)
				}
				suffix := fmt.Sprintf("%d-to-%d-support-%t", tc.source, tc.target, tc.supports)
				for _, pair := range [][2]string{
					{source, "source-" + suffix + ".3mf"},
					{goFinal, "go-" + suffix + ".3mf"},
					{rustFinal, "rust-" + suffix + ".3mf"},
				} {
					exportSlicerFixtureNoOverwrite(t, pair[0], filepath.Join(outputDir, pair[1]))
				}
			}
		})
	}
}

func exportSlicerFixtureNoOverwrite(t *testing.T, source, destination string) {
	t.Helper()
	input, err := os.Open(source)
	if err != nil {
		t.Fatal(err)
	}
	defer input.Close()
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatalf("cannot safely write scratch slicer fixture %s: %v", destination, err)
	}
	defer output.Close()
	if _, err := io.Copy(output, input); err != nil {
		t.Fatal(err)
	}
	if err := output.Sync(); err != nil {
		t.Fatal(err)
	}
}

// All modified XML packages may differ lexically between Go's raw token
// reserializer and Rust's quick-xml writer, while retaining the same
// semantic element/attribute/text token sequence.
func compareConverted3MFParts(t *testing.T, goPath, rustPath string) {
	t.Helper()
	goParts := inspectZipEntries(t, goPath)
	rustParts := inspectZipEntries(t, rustPath)
	if len(goParts) != len(rustParts) {
		t.Fatalf("ZIP part count Go=%d Rust=%d", len(goParts), len(rustParts))
	}
	for name, want := range goParts {
		got, ok := rustParts[name]
		if !ok {
			t.Fatalf("Rust omitted ZIP part %s", name)
		}
		switch {
		case name == "Metadata/project_settings.config":
			var a, b any
			if err := json.Unmarshal(want, &a); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(got, &b); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(a, b) {
				t.Fatalf("JSON settings differ: Go=%s Rust=%s", want, got)
			}
		case name == "[Content_Types].xml" || strings.HasSuffix(name, ".rels") ||
			name == "Metadata/slice_info.config" || name == "Metadata/model_settings.config" ||
			strings.HasSuffix(name, ".model"):
			if !reflect.DeepEqual(semanticXMLTokens(t, want), semanticXMLTokens(t, got)) {
				t.Fatalf("XML differs in %s\nGo: %s\nRust: %s", name, want, got)
			}
		default:
			if !bytes.Equal(want, got) {
				t.Fatalf("unchanged payload differs at part %s", name)
			}
		}
	}
}

func TestRustFullArchiveRejectsUnverifiedSources(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	for _, scenario := range []string{
		"stale_project_settings_snapshot",
		"incomplete_source_mapping",
		"invented_source_id",
		"mapped_to_unallocated_slot",
		"merged_distinct_material_slots",
		"unsafe_machine_script_override",
		"unsafe_material_temperature_override",
		"source_palette_snapshot_mismatch",
		"profile_slot_count_mismatch",
		"empty_material_profile_id",
		"noncanonical_source_id",
		"empty_material_type",
		"undeclared_default_extruder",
		"undeclared_painted_filament",
	} {
		t.Run(scenario, func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			destination := filepath.Join(dir, "must-not-exist.3mf")
			fixture(t, source, 2)

			info, err := converter.Inspect(source)
			if err != nil {
				t.Fatal(err)
			}
			slots, mapping, err := sourceSlotPlan(info.Filaments)
			if err != nil {
				t.Fatal(err)
			}
			profile := printer.Builtin("snapmaker-u1")
			originalSettings := map[string]any{}
			if err := json.Unmarshal(
				inspectZipEntries(t, source)["Metadata/project_settings.config"],
				&originalSettings,
			); err != nil {
				t.Fatal(err)
			}
			sourceMap := map[string]int{}
			for id, slot := range mapping {
				sourceMap[strconv.Itoa(id)] = slot
			}
			plan := map[string]any{
				"profile": profile, "slots": slots, "slot_count": 4,
				"supports": false, "source_settings": originalSettings,
				"mapping": sourceMap,
			}

			switch scenario {
			case "stale_project_settings_snapshot":
				stale := map[string]any{}
				for key, value := range originalSettings {
					stale[key] = value
				}
				stale["printer_model"] = "INVENTED_STALE_PRINTER"
				plan["source_settings"] = stale
			case "incomplete_source_mapping":
				delete(sourceMap, "2")
			case "invented_source_id":
				sourceMap["99"] = 1
			case "mapped_to_unallocated_slot":
				sourceMap["1"] = 3 // printer has four bays, but only two declared materials
			case "merged_distinct_material_slots":
				sourceMap["2"] = sourceMap["1"] // preserve two source IDs; merging is forbidden
			case "unsafe_machine_script_override":
				plan["preserved"] = []string{"machine_start_gcode"}
			case "unsafe_material_temperature_override":
				plan["preserved"] = []string{"nozzle_temperature"}
			case "source_palette_snapshot_mismatch":
				plan["source_filaments"] = []map[string]any{
					{"id": 1, "type": "ABS", "color": "#FF00FF"},
					{"id": 2, "type": "PLA", "color": "#00FF00"},
				}
			case "profile_slot_count_mismatch":
				plan["slot_count"] = 5
			case "empty_material_profile_id":
				profile.FilamentProfiles["PLA"] = ""
			case "noncanonical_source_id":
				sourceMap["01"] = 1
			case "empty_material_type":
				slots[0].Type = ""
			case "undeclared_default_extruder":
				replaceZIPPart(t, source, "Metadata/model_settings.config",
					[]byte("<config><metadata key=\"extruder\" value=\"99\"/></config>"))
			case "undeclared_painted_filament":
				parts := inspectZipEntries(t, source)
				injected := false
				for name, xml := range parts {
					if !strings.HasSuffix(name, ".model") || !bytes.Contains(xml, []byte("<triangle")) {
						continue
					}
					changed := bytes.Replace(xml, []byte("<triangle"),
						[]byte("<triangle paint_color=\"DC\""), 1)
					replaceZIPPart(t, source, name, changed)
					injected = true
					break
				}
				if !injected {
					t.Fatal("fixture had no editable triangle for stale paint-ID check")
				}
			}
			bytesBefore, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			planBytes, err := json.Marshal(plan)
			if err != nil {
				t.Fatal(err)
			}
			planFile := filepath.Join(dir, "plan.json")
			if err := os.WriteFile(planFile, planBytes, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()
			output, err := exec.CommandContext(ctx, rust,
				"--convert-archive", planFile, source, destination).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			if err == nil {
				t.Fatalf("unsafe plan unexpectedly accepted: %s", output)
			}
			var reply map[string]any
			if parseErr := json.Unmarshal(output, &reply); parseErr != nil {
				t.Fatalf("invalid Rust rejection JSON: %v %q", parseErr, output)
			}
			if reply["ok"] != false {
				t.Fatalf("unsafe plan not clearly rejected: %v", reply)
			}
			if _, statErr := os.Stat(destination); !os.IsNotExist(statErr) {
				t.Fatalf("unsafe converted output appeared: %v", statErr)
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(bytesBefore, after) {
				t.Fatal("unsafe request changed the source 3MF")
			}
			if temporaries, _ := filepath.Glob(filepath.Join(dir, ".slicebridge*")); len(temporaries) != 0 {
				t.Fatalf("temporary outputs left behind after rejection: %v", temporaries)
			}
		})
	}
}
