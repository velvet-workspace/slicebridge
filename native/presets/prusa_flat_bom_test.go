package presets

import (
	"path/filepath"
	"testing"
)

// The strict INI reader handles a BOM only at offset zero. Wrapping a
// trusted flat user preset must keep that exact allowance, never loosen it.
func TestPrusaFlatUserSingleLeadingUTF8BOM(t *testing.T) {
	loc, _ := prusaFixture(t)
	file := filepath.Join(loc.user, "printer", "QIDI Plus5 0.4 nozzle.ini")
	writePrusaFile(t, file, "\ufeffinherits = X-Plus 4 0.4 nozzle\r\n")
	found := discoverPrusa(loc)
	if len(found.Targets) != 1 || found.Targets[0].Nozzle != "0.4" || len(found.Rejected) != 0 {
		t.Fatalf("valid BOM+CRLF trusted flat profile rejected: %+v", found)
	}
	// Vendor bundles remain section-only, and ambiguous BOMs remain rejected.
	if _, err := parsePrusaIndexedFile(file, false, []byte("\ufeffinherits = X-Plus 4 0.4 nozzle\n")); err == nil {
		t.Fatal("vendor flat BOM unexpectedly accepted")
	}
	for _, raw := range []string{
		"\ufeff\ufeffinherits = X-Plus 4 0.4 nozzle\n",
		"\ufeffinherits = X-Plus 4 0.4 nozzle\n[printer:Injected]\nnozzle_diameter = 0.4\n",
	} {
		writePrusaFile(t, file, raw)
		if got := discoverPrusa(loc); len(got.Targets) != 0 {
			t.Fatalf("malformed or double-BOM file accepted: %+v", got.Targets)
		}
	}
}
