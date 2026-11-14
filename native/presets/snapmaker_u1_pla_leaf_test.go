package presets

import (
	"path/filepath"
	"testing"
)

func TestInstalledSnapmakerU1PLA04LeafRequiresFullOEMMaterialLimits(t *testing.T) {
	sample := func() catalog {
		return catalog{
			"filament:Generic PLA High Speed @U1 base": {{
				file: filepath.Join("Snapmaker", "filament", "Generic PLA High Speed @U1 base.json"),
				settings: map[string]any{
					"filament_type": []any{"PLA"},
				},
			}},
			"filament:Generic PLA High Speed": {{
				file: filepath.Join("Snapmaker", "filament", "Generic PLA High Speed.json"),
				settings: map[string]any{
					"inherits":                      "Generic PLA High Speed @U1 base",
					"filament_type":                 []any{"PLA"},
					"compatible_printers":           []any{"Snapmaker U1 (0.4 nozzle)"},
					"nozzle_temperature":            []any{"220"},
					"nozzle_temperature_range_low":  []any{"190"},
					"nozzle_temperature_range_high": []any{"240"},
					"filament_flow_ratio":           []any{"0.98"},
					"filament_max_volumetric_speed": []any{"18"},
				},
			}},
		}
	}
	const machine = "Snapmaker U1 (0.4 nozzle)"
	if got := installedU104MaterialLeaf(sample(), machine, "PLA"); got != "Generic PLA High Speed" {
		t.Fatalf("valid OEM U1 PLA leaf not chosen: %q", got)
	}
	cases := []struct {
		name   string
		mutate func(catalog)
	}{
		{"wrong target nozzle", func(c catalog) {
			c["filament:Generic PLA High Speed"][0].settings["compatible_printers"] = []any{"Snapmaker U1 (0.6 nozzle)"}
		}},
		{"missing flow", func(c catalog) { delete(c["filament:Generic PLA High Speed"][0].settings, "filament_flow_ratio") }},
		{"missing temperature", func(c catalog) { delete(c["filament:Generic PLA High Speed"][0].settings, "nozzle_temperature") }},
		{"missing max volumetric speed", func(c catalog) {
			delete(c["filament:Generic PLA High Speed"][0].settings, "filament_max_volumetric_speed")
		}},
		{"wrong lineage", func(c catalog) { c["filament:Generic PLA High Speed"][0].settings["inherits"] = "Unverified PLA Base" }},
		{"wrong type", func(c catalog) { c["filament:Generic PLA High Speed"][0].settings["filament_type"] = []any{"PETG"} }},
		{"wrong filename", func(c catalog) {
			c["filament:Generic PLA High Speed"][0].file = filepath.Join("Snapmaker", "filament", "PLA Unknown.json")
		}},
		{"too high flow", func(c catalog) {
			c["filament:Generic PLA High Speed"][0].settings["filament_flow_ratio"] = []any{"2.5"}
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			input := sample()
			tc.mutate(input)
			if got := installedU104MaterialLeaf(input, machine, "PLA"); got != "" {
				t.Fatalf("invalid OEM PLA preset accepted: %q", got)
			}
		})
	}
	if got := installedU104MaterialLeaf(sample(), machine, "ABS"); got != "" {
		t.Fatalf("unsupported material resolved to U1 leaf: %q", got)
	}
}
