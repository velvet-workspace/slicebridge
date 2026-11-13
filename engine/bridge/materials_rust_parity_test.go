package bridge

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// runRustMaterialCommand passes a bounded synthetic request as a temporary
// JSON file. This never runs any real slicer, printer, or native application.
func runRustMaterialCommand(t *testing.T, binary, mode string, request any) (map[string]any, error) {
	t.Helper()
	encoded, err := json.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	if len(encoded) > 8<<20 {
		t.Fatal("request too large")
	}
	filename := filepath.Join(t.TempDir(), "material-request.json")
	if err := os.WriteFile(filename, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, mode, filename)
	output, cmdErr := cmd.Output()
	if ctx.Err() != nil {
		t.Fatal(ctx.Err())
	}
	var result map[string]any
	if err := json.Unmarshal(output, &result); err != nil {
		t.Fatalf("Rust JSON invalid: %v (exit=%v stdout=%q)", err, cmdErr, output)
	}
	if _, ok := result["ok"].(bool); !ok {
		t.Fatalf("Rust omitted a boolean ok flag: %v", result)
	}
	return result, cmdErr
}

func parityJSON(t *testing.T, want, got any) {
	t.Helper()
	left, err := json.Marshal(want)
	if err != nil {
		t.Fatal(err)
	}
	right, err := json.Marshal(got)
	if err != nil {
		t.Fatal(err)
	}
	var decodedLeft, decodedRight any
	if err := json.Unmarshal(left, &decodedLeft); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(right, &decodedRight); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(decodedLeft, decodedRight) {
		t.Fatalf("Go/Rust data differs\nGo: %s\nRust: %s", left, right)
	}
}

func verifySourceSlotPlanParity(t *testing.T, rust string, filaments []converter.Filament) {
	t.Helper()
	goSlots, mapping, goErr := sourceSlotPlan(filaments)
	reply, rustErr := runRustMaterialCommand(t, rust, "--plan-slots", map[string]any{"filaments": filaments})
	if goErr != nil {
		if rustErr == nil || reply["ok"] == true {
			t.Fatalf("Go refused slot plan (%v) but Rust accepted (%v)", goErr, reply)
		}
		return
	}
	if rustErr != nil || reply["ok"] != true {
		t.Fatalf("Go accepted slot plan, Rust refused: %v %v", rustErr, reply)
	}
	want := map[string]any{"slots": goSlots, "mapping": mapping}
	parityJSON(t, want, reply["plan"])
}

func TestRustSourceSlotPlanParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	var large []converter.Filament
	for i := 0; i < 65; i++ {
		large = append(large, converter.Filament{
			ID: i + 1, Type: "PLA", Color: "#FFFFFF",
		})
	}
	for _, tc := range []struct {
		name      string
		filaments []converter.Filament
	}{
		{"one", []converter.Filament{{ID: 1, Color: "#FFFFFF", Type: "PLA"}}},
		{"identical_colors_stay_distinct", []converter.Filament{
			{ID: 4, Color: "#00FF00", Type: "PLA"},
			{ID: 6, Color: "#00FF00", Type: "PLA"},
		}},
		{"support_material", []converter.Filament{
			{ID: 7, Color: "#EEEEEE", Type: "PVA"},
			{ID: 9, Color: "#CC0000", Type: "PETG"},
			{ID: 11, Color: "#000000", Type: "PLA breakaway"},
		}},
		{"support_edge_cases", []converter.Filament{
			{ID: 1, Color: "#000000", Type: " pVa "},
			{ID: 2, Color: "#111111", Type: "Support for PLA"},
			{ID: 3, Color: "#222222", Type: "HIPS"},
			{ID: 4, Color: "#333333", Type: "PLA"},
		}},
		{"out_of_order_ids", []converter.Filament{
			{ID: 40, Color: "#FFFFFF", Type: "ABS"},
			{ID: 1, Color: "#111111", Type: "PETG"},
			{ID: 17, Color: "#222222", Type: "PLA"},
		}},
		{"empty", nil},
		{"duplicate", []converter.Filament{
			{ID: 1, Color: "#111111", Type: "PLA"},
			{ID: 1, Color: "#222222", Type: "PETG"},
		}},
		{"duplicate_before_missing_color", []converter.Filament{
			{ID: 1, Color: "#111111", Type: "PLA"},
			{ID: 1, Color: "", Type: "PETG"},
		}},
		{"invalid_id", []converter.Filament{{ID: 0, Color: "#111111", Type: "PLA"}}},
		{"missing_color", []converter.Filament{{ID: 1, Color: "", Type: "PLA"}}},
		{"missing_material", []converter.Filament{{ID: 1, Color: "#111111", Type: ""}}},
		{"oversized_palette", large},
	} {
		t.Run(tc.name, func(t *testing.T) {
			verifySourceSlotPlanParity(t, rust, tc.filaments)
		})
	}
	sources, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "*.3mf"))
	if err != nil || len(sources) == 0 {
		t.Fatalf("missing test fixtures: %v", err)
	}
	for _, source := range sources {
		t.Run("fixture_"+filepath.Base(source), func(t *testing.T) {
			inspection, err := converter.Inspect(source)
			if err != nil {
				t.Fatalf("Go source fixture inspection failed: %v", err)
			}
			verifySourceSlotPlanParity(t, rust, inspection.Filaments)
		})
	}
}

func verifyMaterialMapParity(t *testing.T, rust string, bundle *targetBundle, filaments []converter.Filament) {
	t.Helper()
	input := map[string]any{"bundle": bundle, "filaments": filaments}
	// Rust must receive the original bundle, before the Go comparison mutates it.
	reply, rustErr := runRustMaterialCommand(t, rust, "--map-materials", input)
	goErr := bindNativeFilamentSettings(bundle, filaments)
	if goErr != nil {
		if rustErr == nil || reply["ok"] == true {
			t.Fatalf("Go refused material map (%v) but Rust accepted (%v)", goErr, reply)
		}
		return
	}
	if rustErr != nil || reply["ok"] != true {
		t.Fatalf("Go accepted material map, Rust refused: %v %v", rustErr, reply)
	}
	parityJSON(t, bundle.Profile.ProjectSettings, reply["project_settings"])
}

func TestRustNativeMaterialMappingParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	for _, tc := range []struct {
		name      string
		filaments []converter.Filament
		mutate    func(*targetBundle)
	}{
		{"mixed_PLA_PETG_ABS", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"}, {ID: 3, Type: "ABS"},
		}, nil},
		{"identical_materials_stay_separate", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PLA"}, {ID: 3, Type: "PETG"},
		}, nil},
		{"installed_name_map_recovers_blank_or_missing_filament_settings_id", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
		}, func(bundle *targetBundle) {
			bundle.MaterialSettings["PLA"]["filament_settings_id"] = []any{""}
			delete(bundle.MaterialSettings["PETG"], "filament_settings_id")
		}},
		{"hardware_keys_owned_by_machine", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
		}, func(bundle *targetBundle) {
			bundle.MaterialSettings["PLA"]["nozzle_diameter"] = []any{"0.2"}
			bundle.MaterialSettings["PETG"]["nozzle_diameter"] = []any{"0.6"}
			bundle.MaterialSettings["PLA"]["machine_max_acceleration_extruding"] = []any{"100"}
		}},
		{"missing_critical_native_PETG_lower_temperature_bound", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
		}, func(bundle *targetBundle) {
			bundle.MaterialSettings["PLA"]["nozzle_temperature_range_low"] = []any{"190"}
			delete(bundle.MaterialSettings["PETG"], "nozzle_temperature_range_low")
		}},
		{"missing_critical_nozzle_temperature", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
		}, func(bundle *targetBundle) {
			delete(bundle.MaterialSettings["PETG"], "nozzle_temperature")
		}},
		{"missing_critical_flow_ratio", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
		}, func(bundle *targetBundle) {
			delete(bundle.MaterialSettings["PETG"], "filament_flow_ratio")
		}},
		{"whitespace_material_key_preserved_then_trimmed_in_output", []converter.Filament{
			{ID: 1, Type: " PLA "},
		}, func(bundle *targetBundle) {
			bundle.Profile.FilamentProfiles[" PLA "] = "Native PLA With Whitespace"
			bundle.MaterialSettings[" PLA "] = bundle.MaterialSettings["PLA"]
		}},
		{"unknown_material", []converter.Filament{{ID: 1, Type: "PLA-CF"}}, nil},
		{"missing_installed_material_profile", []converter.Filament{{ID: 1, Type: "ABS"}}, func(bundle *targetBundle) {
			delete(bundle.Profile.FilamentProfiles, "ABS")
		}},
		{"no_filaments", nil, nil},
		{"more_filaments_than_profile_slots", []converter.Filament{
			{ID: 1, Type: "PLA"}, {ID: 2, Type: "PLA"}, {ID: 3, Type: "PLA"},
			{ID: 4, Type: "PLA"}, {ID: 5, Type: "PLA"},
		}, nil},
		{"scalar_values_only", []converter.Filament{{ID: 1, Type: "PLA"}}, func(bundle *targetBundle) {
			bundle.MaterialSettings["PLA"] = map[string]any{"comment": "installed"}
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			bundle := syntheticMaterialBundle()
			if tc.mutate != nil {
				tc.mutate(bundle)
			}
			verifyMaterialMapParity(t, rust, bundle, tc.filaments)
		})
	}

	// Exercise material mapping against the real source fixture channel counts,
	// with clearly synthetic installed presets. These are not real slicer profiles.
	sources, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "*.3mf"))
	if err != nil || len(sources) == 0 {
		t.Fatalf("missing test fixtures: %v", err)
	}
	for _, source := range sources {
		t.Run("fixture_"+filepath.Base(source), func(t *testing.T) {
			inspection, err := converter.Inspect(source)
			if err != nil {
				t.Fatal(err)
			}
			bundle := syntheticMaterialBundle()
			if len(inspection.Filaments) > bundle.Profile.FilamentSlots {
				bundle.Profile.FilamentSlots = len(inspection.Filaments)
			}
			for _, f := range inspection.Filaments {
				if _, ok := bundle.Profile.FilamentProfiles[f.Type]; !ok {
					bundle.Profile.FilamentProfiles[f.Type] = "Synthetic " + f.Type
					bundle.MaterialSettings[f.Type] = map[string]any{
						"nozzle_temperature":   []any{"210"},
						"filament_flow_ratio":  []any{"0.99"},
						"filament_type":        []any{f.Type},
						"filament_settings_id": []any{"Synthetic " + f.Type},
					}
				}
			}
			verifyMaterialMapParity(t, rust, bundle, inspection.Filaments)
		})
	}
}
