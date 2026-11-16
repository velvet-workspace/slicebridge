package presets

import (
    "path/filepath"
    "testing"
)

// Flat user .ini exports are valid PrusaSlicer inputs. They must still pass
// the same full printer/print/PLA checks as explicit sectioned profiles.
func TestDiscoverPrusaFlatUserPrinterPreservesSafety(t *testing.T) {
    loc, _ := prusaFixture(t)
    filename := filepath.Join(loc.user, "printer", "QIDI Plus5 0.4 nozzle.ini")
    writePrusaFile(t, filename, "inherits = X-Plus 4 0.4 nozzle\n")
    found := discoverPrusa(loc)
    if len(found.Rejected) != 0 {
        t.Fatalf("flat valid source rejected: %+v", found.Rejected)
    }
    if len(found.Targets) != 1 || found.Targets[0].Title != "QIDI Plus5 0.4 nozzle" ||
        found.Targets[0].Nozzle != "0.4" {
        t.Fatalf("expected one QIDI printer, got %+v", found.Targets)
    }

    // Nozzle not in the known matrix must never be offered.
    writePrusaFile(t, filename, "inherits = X-Plus 4 0.4 nozzle\nnozzle_diameter = 0.77\n")
    if got := discoverPrusa(loc); len(got.Targets) != 0 {
        t.Fatalf("invalid nozzle became ready: %+v", got.Targets)
    }

    // Never rescue malformed sectioned user files as flat profiles.
    writePrusaFile(t, filename, "inherits = X-Plus 4 0.4 nozzle\n[printer:Injected]\nnozzle_diameter = 0.4\n")
    if got := discoverPrusa(loc); len(got.Targets) != 0 {
        t.Fatalf("mixed malformed file became ready: %+v", got.Targets)
    }
}
