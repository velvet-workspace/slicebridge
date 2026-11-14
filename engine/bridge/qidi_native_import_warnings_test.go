package bridge

import (
	"strings"
	"testing"
)

func TestQIDINativeMultiMaterialImportWarning(t *testing.T) {
	warnings := vendorNativeImportWarnings("qidi", 5)
	if len(warnings) != 1 {
		t.Fatalf("QIDI multi-material native GUI warning missing: %v", warnings)
	}
	message := warnings[0]
	for _, required := range []string{
		"QIDIStudio", "replace printer and material settings",
		"5 separate virtual material slots", "one local filament preset",
		"Verify that all 5 material profiles", "geometry-only",
		"does not claim native QIDI multi-material GUI acceptance",
	} {
		if !strings.Contains(message, required) {
			t.Fatalf("warning omitted %q: %s", required, message)
		}
	}
	for _, tc := range []struct {
		target string
		slots  int
	}{
		{"qidi", 1},
		{"u1", 5},
		{"c5", 5},
		{"qidi", 0},
	} {
		if got := vendorNativeImportWarnings(tc.target, tc.slots); len(got) != 0 {
			t.Fatalf("false native GUI warning for %s/%d: %v", tc.target, tc.slots, got)
		}
	}
}
