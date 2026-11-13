package presets

import (
	"path/filepath"
	"testing"
)

func TestInstalledSnapmakerU1PETGLeafRequiresExactMachineAndCompleteOEMTemperatureRange(t *testing.T) {
	snapshot := func() catalog {
		return catalog{
			"filament:Generic PETG @U1 base": {{
				file: filepath.Join("Snapmaker", "filament", "Generic PETG @U1 base.json"),
				settings: map[string]any{
					"filament_type":                 []any{"PETG"},
					"nozzle_temperature_range_high": []any{"270"},
				},
			}},
			"filament:Generic PETG": {{
				file: filepath.Join("Snapmaker", "filament", "Generic PETG.json"),
				settings: map[string]any{
					"inherits":                     "Generic PETG @U1 base",
					"filament_type":                []any{"PETG"},
					"nozzle_temperature_range_low": []any{"230"},
					"compatible_printers":          []any{"Snapmaker U1 (0.4 nozzle)"},
				},
			}},
		}
	}
	const machine = "Snapmaker U1 (0.4 nozzle)"
	if got := installedU1PETG04Leaf(snapshot(), machine); got != "Generic PETG" {
		t.Fatalf("verified OEM compatible PETG leaf not selected: %q", got)
	}
	cases := []struct {
		name   string
		mutate func(catalog)
	}{
		{"missing exact OEM machine", func(ix catalog) {
			ix["filament:Generic PETG"][0].settings["compatible_printers"] = []any{"Snapmaker U1 (0.6 nozzle)"}
		}},
		{"missing lower bound", func(ix catalog) {
			delete(ix["filament:Generic PETG"][0].settings, "nozzle_temperature_range_low")
		}},
		{"malformed lower bound", func(ix catalog) {
			ix["filament:Generic PETG"][0].settings["nozzle_temperature_range_low"] = []any{"unknown"}
		}},
		{"inverted temperature range", func(ix catalog) {
			ix["filament:Generic PETG"][0].settings["nozzle_temperature_range_low"] = []any{"280"}
		}},
		{"wrong material", func(ix catalog) {
			ix["filament:Generic PETG"][0].settings["filament_type"] = []any{"PLA"}
		}},
		{"unknown inheritance", func(ix catalog) {
			ix["filament:Generic PETG"][0].settings["inherits"] = "Another Material Base"
		}},
		{"different raw vendor filename", func(ix catalog) {
			ix["filament:Generic PETG"][0].file = filepath.Join("Snapmaker", "filament", "Unverified PETG.json")
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			ix := snapshot()
			tc.mutate(ix)
			if got := installedU1PETG04Leaf(ix, machine); got != "" {
				t.Fatalf("unverified OEM PETG candidate accepted: %q", got)
			}
		})
	}
}
