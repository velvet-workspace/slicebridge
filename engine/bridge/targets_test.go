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
				"nozzle_temperature":           []any{"210"},
				"filament_flow_ratio":          []any{"0.98"},
				"filament_type":                []any{"PLA"},
				"filament_settings_id":         []any{"Native PLA"},
				"additional_cooling_fan_speed": []any{"60"},
			},
			"PETG": {
				"nozzle_temperature":   []any{"240"},
				"filament_flow_ratio":  []any{"0.97"},
				"filament_type":        []any{"PETG"},
				"filament_settings_id": []any{"Native PETG"},
			},
			"ABS": {
				"nozzle_temperature":           []any{"255"},
				"filament_flow_ratio":          []any{"0.99"},
				"filament_type":                []any{"ABS"},
				"filament_settings_id":         []any{"Native ABS"},
				"additional_cooling_fan_speed": []any{"20"},
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
