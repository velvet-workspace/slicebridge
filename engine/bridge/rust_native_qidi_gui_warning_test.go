package bridge

import (
	"strings"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// Captures the actual QIDIStudio 02.07.02.73 Windows GUI observation:
// a correctly typed fifth PETG virtual filament is initially presented as
// PLA Rapido on BOTH current Go and Rust project imports. The warning must
// remain narrow and cannot imply a five-tool X-Plus 5 or change stable Go.
func TestRustNativeQIDIXPlus5MixedPETGImportWarning(t *testing.T) {
	slots := []converter.Slot{
		{Type: "PLA", Color: "#FF0000"},
		{Type: "PLA", Color: "#00FF00"},
		{Type: "PLA", Color: "#0000FF"},
		{Type: "PLA", Color: "#FFFF00"},
		{Type: "PETG", Color: "#FFFFFF"},
	}
	warning := qidiXPlus5PETGImportWarning("qidi", "X-Plus 5", 1, slots)
	for _, wanted := range []string{"PETG", "PLA Rapido", "slot 5", "color", "physical toolheads"} {
		if !strings.Contains(warning, wanted) {
			t.Fatalf("QIDI GUI import caution must mention %q: %s", wanted, warning)
		}
	}
	// Avoid misleading warnings for unaffected profiles or material patterns.
	for _, tc := range []struct {
		name   string
		target string
		model  string
		tools  int
		slots  []converter.Slot
	}{
		{"Snapmaker uses correct PETG GUI preset", "u1", "Snapmaker U1", 4, slots},
		{"QIDI different model not verified", "qidi", "X-Max 4", 1, slots},
		{"QIDI unexpected tool configuration", "qidi", "X-Plus 5", 4, slots},
		{"QIDI one material", "qidi", "X-Plus 5", 1, slots[:1]},
		{"QIDI four virtual materials", "qidi", "X-Plus 5", 1, slots[:4]},
		{"QIDI fifth slot not PETG", "qidi", "X-Plus 5", 1, append(append([]converter.Slot{}, slots[:4]...), converter.Slot{Type: "PLA"})},
		{"QIDI non-PLA source", "qidi", "X-Plus 5", 1, append([]converter.Slot{{Type: "ABS"}}, slots[1:]...)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if warning := qidiXPlus5PETGImportWarning(tc.target, tc.model, tc.tools, tc.slots); warning != "" {
				t.Fatalf("unverified profile got specific QIDI warning: %s", warning)
			}
		})
	}
}
