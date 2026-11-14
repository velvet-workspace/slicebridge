package presets

import (
	"os"
	"testing"
)

func TestQAInstalledSnapmakerU106MaterialLeaves(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("opt-in installed OEM read-only test")
	}
	b, err := LoadConfigured("u1", "0.6", "auto", nil)
	if err != nil {
		t.Fatalf("installed U1 0.6 native profile incomplete: %v", err)
	}
	for _, typ := range []string{"PLA", "PETG"} {
		leaf := b.Profile.FilamentProfiles[typ]
		settings := b.MaterialSettings[typ]
		t.Logf("U1_06_OEM material=%s selected_leaf=%q temperature=%v flow=%v volumetric=%v", typ, leaf, settings["nozzle_temperature"], settings["filament_flow_ratio"], settings["filament_max_volumetric_speed"])
		if leaf == "" || settings == nil {
			t.Fatalf("missing verified 0.6 native leaf %s", typ)
		}
	}
}
