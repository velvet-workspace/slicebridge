package bridge

import (
	"reflect"
	"strings"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

func syntheticMaterialBundle() *targetBundle {
	return &targetBundle{
		Target: "c5",
		Profile: printer.Profile{
			Name: "c5", DisplayName: "Flashforge Creator 5", ModelID: "Flashforge Creator 5",
			FilamentSlots: 4,
			FilamentProfiles: map[string]string{
				"PLA": "Native PLA", "PETG": "Native PETG", "ABS": "Native ABS",
			},
			ProjectSettings: map[string]any{
				"printer_model":                "Flashforge Creator 5",
				"nozzle_diameter":              []any{"0.4", "0.4", "0.4", "0.4"},
				"machine_start_gcode":          "G28 ; native",
				"machine_end_gcode":            "M104 S0 ; native",
				"additional_cooling_fan_speed": []any{"60"},
			},
		},
		MaterialSettings: map[string]map[string]any{
			"PLA": {
				"nozzle_temperature":            []any{"210"},
				"filament_flow_ratio":           []any{"0.98"},
				"filament_max_volumetric_speed": []any{"18"},
				"filament_type":                 []any{"PLA"},
				"filament_settings_id":          []any{"Native PLA"},
				"additional_cooling_fan_speed":  []any{"60"},
			},
			"PETG": {
				"nozzle_temperature":            []any{"240"},
				"filament_flow_ratio":           []any{"0.97"},
				"filament_max_volumetric_speed": []any{"12"},
				"filament_type":                 []any{"PETG"},
				"filament_settings_id":          []any{"Native PETG"},
			},
			"ABS": {
				"nozzle_temperature":            []any{"255"},
				"filament_flow_ratio":           []any{"0.99"},
				"filament_max_volumetric_speed": []any{"16"},
				"filament_type":                 []any{"ABS"},
				"filament_settings_id":          []any{"Native ABS"},
				"additional_cooling_fan_speed":  []any{"20"},
			},
		},
	}
}

func TestNativeFilamentSettingsMapEverySlot(t *testing.T) {
	bundle := syntheticMaterialBundle()
	filaments := []converter.Filament{
		{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"}, {ID: 3, Type: "ABS"},
	}
	if err := bindNativeFilamentSettings(bundle, filaments); err != nil {
		t.Fatal(err)
	}
	cfg := bundle.Profile.ProjectSettings
	wantTemp := []any{"210", "240", "255"}
	wantFlow := []any{"0.98", "0.97", "0.99"}
	if !reflect.DeepEqual(cfg["nozzle_temperature"], wantTemp) {
		t.Fatalf("wrong material temperatures: %v", cfg["nozzle_temperature"])
	}
	if !reflect.DeepEqual(cfg["filament_flow_ratio"], wantFlow) {
		t.Fatalf("wrong material flow rates: %v", cfg["filament_flow_ratio"])
	}
	if !reflect.DeepEqual(cfg["filament_type"], []any{"PLA", "PETG", "ABS"}) {
		t.Fatalf("wrong material names: %v", cfg["filament_type"])
	}
	if _, exists := cfg["additional_cooling_fan_speed"]; exists {
		t.Fatal("PLA-only override leaked onto PETG; optional key must be omitted")
	}
	if got := cfg["machine_start_gcode"]; got != "G28 ; native" {
		t.Fatalf("machine-owned firmware script changed: %v", got)
	}
}

func TestNativeFilamentProfileIDsFollowVerifiedInstalledMapEvenWhenMaterialPresetOmitsID(t *testing.T) {
	bundle := syntheticMaterialBundle()
	bundle.Target = "qidi"
	bundle.Profile.ModelID = "X-Plus 5"
	// An installed target's one material provides a blank
	// filament_settings_id placeholder and PETG omits the key completely.
	// The independently resolved installed FilamentProfiles mapping retains
	// both exact OEM names and must never silently substitute PLA for PETG.
	bundle.MaterialSettings["PLA"]["filament_settings_id"] = []any{""}
	delete(bundle.MaterialSettings["PETG"], "filament_settings_id")
	originalPLA := bundle.Profile.FilamentProfiles["PLA"]
	originalPETG := bundle.Profile.FilamentProfiles["PETG"]
	err := bindNativeFilamentSettings(bundle, []converter.Filament{
		{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
	})
	if err != nil {
		t.Fatalf("installed native profile map should be authoritative: %v", err)
	}
	wantIDs := []any{originalPLA, originalPETG}
	if !reflect.DeepEqual(bundle.Profile.ProjectSettings["filament_settings_id"], wantIDs) {
		t.Fatalf("native filament IDs lost or substituted: got %v want %v", bundle.Profile.ProjectSettings["filament_settings_id"], wantIDs)
	}
	if !reflect.DeepEqual(bundle.Profile.ProjectSettings["nozzle_temperature"], []any{"210", "240"}) {
		t.Fatal("native PLA/PETG temperatures were not preserved")
	}
	if _, exists := bundle.MaterialSettings["PETG"]["filament_settings_id"]; exists {
		t.Fatal("material mapping must not mutate the installed PETG preset")
	}
	missing := syntheticMaterialBundle()
	missing.Target = "qidi"
	missing.Profile.ModelID = "X-Plus 5"
	delete(missing.Profile.FilamentProfiles, "PETG")
	delete(missing.MaterialSettings["PETG"], "filament_settings_id")
	if err := bindNativeFilamentSettings(missing, []converter.Filament{
		{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
	}); err == nil {
		t.Fatal("missing native PETG profile identity was silently accepted")
	}
}

func TestMissingCriticalMaterialValuesFailClosed(t *testing.T) {
	bundle := syntheticMaterialBundle()
	delete(bundle.MaterialSettings["PETG"], "nozzle_temperature")
	err := bindNativeFilamentSettings(bundle, []converter.Filament{
		{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
	})
	if err == nil || !strings.Contains(err.Error(), "nozzle_temperature") {
		t.Fatalf("expected missing temperature refusal, got %v", err)
	}
}

func TestAllNativeMaterialPresetsMissingCriticalValuesFailClosed(t *testing.T) {
	for _, key := range []string{"nozzle_temperature", "filament_flow_ratio", "filament_max_volumetric_speed"} {
		t.Run(key, func(t *testing.T) {
			bundle := syntheticMaterialBundle()
			delete(bundle.MaterialSettings["PLA"], key)
			delete(bundle.MaterialSettings["PETG"], key)
			err := bindNativeFilamentSettings(bundle, []converter.Filament{
				{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
			})
			if err == nil || !strings.Contains(err.Error(), key) {
				t.Fatalf("missing every installed %s value must refuse conversion, got %v", key, err)
			}
			if _, changed := bundle.Profile.ProjectSettings["nozzle_temperature"]; changed {
				t.Fatal("invalid material mapping mutated project settings before refusing")
			}
		})
	}
}

func TestBlankOrNullNativeTemperatureFailsClosed(t *testing.T) {
	for _, value := range []any{"", "  ", nil, float64(0), "0", "-5", "abc", "NaN", "Infinity"} {
		bundle := syntheticMaterialBundle()
		bundle.MaterialSettings["PLA"]["nozzle_temperature"] = []any{value}
		err := bindNativeFilamentSettings(bundle, []converter.Filament{{ID: 1, Type: "PLA"}})
		if err == nil || !strings.Contains(err.Error(), "nozzle_temperature") {
			t.Fatalf("unsafe native temperature value %v was accepted: %v", value, err)
		}
	}
}

func TestNativeFlowRatioAliasFromInstalledPreset(t *testing.T) {
	bundle := syntheticMaterialBundle()
	bundle.MaterialSettings["PLA"]["flow_ratio"] = bundle.MaterialSettings["PLA"]["filament_flow_ratio"]
	delete(bundle.MaterialSettings["PLA"], "filament_flow_ratio")
	if err := bindNativeFilamentSettings(bundle, []converter.Filament{{ID: 1, Type: "PLA"}}); err != nil {
		t.Fatalf("installed flow_ratio alias should remain supported: %v", err)
	}
}

func TestPartialNativePlateTemperatureFailsClosed(t *testing.T) {
	bundle := syntheticMaterialBundle()
	bundle.MaterialSettings["PLA"]["textured_plate_temp"] = []any{"60"}
	filaments := []converter.Filament{{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"}}
	if err := bindNativeFilamentSettings(bundle, filaments); err == nil || !strings.Contains(err.Error(), "textured_plate_temp") {
		t.Fatalf("a PLA-only bed temperature must not leak to PETG: %v", err)
	}
	bundle.MaterialSettings["PETG"]["textured_plate_temp"] = []any{"75"}
	if err := bindNativeFilamentSettings(bundle, filaments); err != nil {
		t.Fatalf("complete installed plate temperatures must map: %v", err)
	}
	if !reflect.DeepEqual(bundle.Profile.ProjectSettings["textured_plate_temp"], []any{"60", "75"}) {
		t.Fatalf("per-material plate values not preserved: %v", bundle.Profile.ProjectSettings["textured_plate_temp"])
	}
}

func TestProfileGuardRejectsUnknownMaterial(t *testing.T) {
	bundle := syntheticMaterialBundle()
	err := bindNativeFilamentSettings(bundle, []converter.Filament{
		{ID: 1, Type: "PLA-CF"},
	})
	if err == nil || !strings.Contains(err.Error(), "PLA-CF") {
		t.Fatalf("unlisted material silently accepted: %v", err)
	}
}

func TestNonQIDIMaterialIDsRemainFromInstalledMaterialPresets(t *testing.T) {
	for _, target := range []string{"c5", "cc2", "u1"} {
		t.Run(target, func(t *testing.T) {
			bundle := syntheticMaterialBundle()
			bundle.Target = target
			bundle.Profile.FilamentProfiles["PLA"] = "Manufacturer profile mapping PLA"
			bundle.Profile.FilamentProfiles["PETG"] = "Manufacturer profile mapping PETG"
			bundle.MaterialSettings["PLA"]["filament_settings_id"] = []any{"Preset PLA"}
			bundle.MaterialSettings["PETG"]["filament_settings_id"] = []any{"Preset PETG"}
			err := bindNativeFilamentSettings(bundle, []converter.Filament{
				{ID: 1, Type: "PLA"}, {ID: 2, Type: "PETG"},
			})
			if err != nil {
				t.Fatal(err)
			}
			got := bundle.Profile.ProjectSettings["filament_settings_id"]
			want := []any{"Preset PLA", "Preset PETG"}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("non-QIDI %s material profile IDs changed: got %v, want %v", target, got, want)
			}
		})
	}
}
