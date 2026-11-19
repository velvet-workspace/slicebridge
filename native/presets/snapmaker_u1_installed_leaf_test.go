package presets

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

// TestInstalledU1PETG04LeafProfile is opt-in on a host with an authorized,
// installed Snapmaker Orca OEM preset root. It reads only vendor files and
// checks that LoadConfigured chooses the exact nozzle-compatible PETG leaf,
// not the temperature-incomplete inheritance-only base. No slicer is launched.
func TestInstalledU1PETG04LeafProfile(t *testing.T) {
	root := os.Getenv("SLICEBRIDGE_INSTALLED_U1_PROFILE_ROOT")
	if root == "" {
		t.Skip("provide exact installed Snapmaker Orca resources/profiles directory for read-only native validation")
	}
	leaf := filepath.Join(root, "Snapmaker", "filament", "Generic PETG.json")
	if info, err := os.Stat(leaf); err != nil || !info.Mode().IsRegular() {
		t.Fatalf("installed OEM PETG leaf unavailable at approved root: %v", err)
	}
	t.Setenv("BRIDGE_SLICER_PROFILE_U1", root)
	bundle, err := LoadConfigured("u1", "0.4", "system", nil)
	if err != nil {
		t.Fatalf("installed Snapmaker Orca U1 0.4 vendor resolver: %v", err)
	}
	if bundle.Target != "u1" || bundle.Nozzle != "0.4" ||
		bundle.Profile.ModelID != "Snapmaker U1" ||
		bundle.Profile.FilamentProfiles["PETG"] != "Generic PETG" ||
		bundle.Profile.FilamentProfiles["PLA"] != "Generic PLA High Speed" {
		t.Fatalf("did not resolve exact installed U1 PLA/PETG 0.4mm OEM leaves: target=%s nozzle=%s model=%s PLA=%s PETG=%s",
			bundle.Target, bundle.Nozzle, bundle.Profile.ModelID,
			bundle.Profile.FilamentProfiles["PLA"], bundle.Profile.FilamentProfiles["PETG"])
	}
	for material, expected := range map[string][4]float64{
		"PLA":  {190, 240, 0.98, 18},
		"PETG": {230, 270, 0.95, 12},
	} {
		params := bundle.MaterialSettings[material]
		for i, key := range []string{"nozzle_temperature_range_low", "nozzle_temperature_range_high", "filament_flow_ratio", "filament_max_volumetric_speed"} {
			raw, ok := params[key].([]any)
			if !ok || len(raw) != 1 {
				t.Fatalf("%s OEM key %s missing/ambiguous: %v", material, key, params[key])
			}
			value, err := strconv.ParseFloat(fmt.Sprint(raw[0]), 64)
			if err != nil || value != expected[i] {
				t.Fatalf("unexpected %s OEM %s=%v (expected %v)", material, key, raw, expected[i])
			}
		}
	}
	petg := bundle.MaterialSettings["PETG"]
	if len(petg) == 0 {
		t.Fatal("flattened installed PETG material settings are missing")
	}
	low, ok := petg["nozzle_temperature_range_low"].([]any)
	if !ok || len(low) != 1 {
		t.Fatal("installed compatible PETG leaf has no lower nozzle temperature bound")
	}
	high, ok := petg["nozzle_temperature_range_high"].([]any)
	if !ok || len(high) != 1 {
		t.Fatal("installed compatible PETG leaf has no upper nozzle temperature bound")
	}
	min, minErr := strconv.ParseFloat(fmt.Sprint(low[0]), 64)
	max, maxErr := strconv.ParseFloat(fmt.Sprint(high[0]), 64)
	if minErr != nil || maxErr != nil || !(min > 0 && min < max && max <= 450) {
		t.Fatalf("inconsistent manufacturer temperature range: low=%v high=%v", low, high)
	}
	t.Logf("Installed Snapmaker U1 0.4mm OEM PETG leaf resolved: name=%s, native temperature bounds=%v..%v; no printer or slicer was touched",
		bundle.Profile.FilamentProfiles["PETG"], low[0], high[0])
}
