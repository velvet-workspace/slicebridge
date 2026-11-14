package bridge

import (
	"strings"
	"testing"
)

func TestQIDISingleFilamentNativeImportWarning(t *testing.T) {
	warnings := vendorNativeImportWarnings("qidi", 1)
	if len(warnings) != 1 {
		t.Fatalf("single-spool QIDI import must still require native material review: %v", warnings)
	}
	for _, required := range []string{
		"one physical print tool", "including single-material projects",
		"loaded physical filament", "local material preset",
		"temperature and flow", "geometry-only",
		"does not claim native QIDI GUI material acceptance",
		"official release notes", "non-QIDIStudio 3MF projects",
		"not proof of native QIDIStudio import acceptance",
	} {
		if !strings.Contains(warnings[0], required) {
			t.Fatalf("single-spool warning omitted %q: %s", required, warnings[0])
		}
	}
	if strings.Contains(warnings[0], "QIDI BOX") {
		t.Fatalf("single-filament import must not suggest a missing multi-material feeder: %s", warnings[0])
	}
}

func TestQIDINativeMultiMaterialImportWarning(t *testing.T) {
	warnings := vendorNativeImportWarnings("qidi", 5)
	if len(warnings) != 1 {
		t.Fatalf("QIDI multi-material native GUI warning missing: %v", warnings)
	}
	message := warnings[0]
	for _, required := range []string{
		"QIDI X-Plus 5 has one physical print tool", "Without a verified QIDI BOX",
		"QIDIStudio", "replace printer and material settings",
		"5 separate virtual material slots", "one local filament preset",
		"Verify that all 5 material profiles", "geometry-only",
		"does not claim native QIDI multi-material GUI acceptance",
		"official releases", "not authored by QIDIStudio",
		"active local filament",
	} {
		if !strings.Contains(message, required) {
			t.Fatalf("warning omitted %q: %s", required, message)
		}
	}
	for _, tc := range []struct {
		target string
		slots  int
	}{
		{"u1", 5},
		{"c5", 5},
		{"qidi", 0},
	} {
		if got := vendorNativeImportWarnings(tc.target, tc.slots); len(got) != 0 {
			t.Fatalf("false native GUI warning for %s/%d: %v", tc.target, tc.slots, got)
		}
	}
}
