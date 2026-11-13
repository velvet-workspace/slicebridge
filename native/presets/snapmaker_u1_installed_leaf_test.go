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
		bundle.Profile.FilamentProfiles["PETG"] != "Generic PETG" {
		t.Fatalf("did not select exact OEM U1 PETG 0.4 leaf: target=%s nozzle=%s model=%s preset=%s",
			bundle.Target, bundle.Nozzle, bundle.Profile.ModelID, bundle.Profile.FilamentProfiles["PETG"])
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
