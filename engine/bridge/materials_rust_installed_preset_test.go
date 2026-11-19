package bridge

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// TestRustInstalledPresetSnapshotParity compares the experimental Rust material
// mapper with Go on snapshots derived from *installed* slicer filament JSON.
// It is opt-in because snapshots are host-specific, not repository fixtures.
// This checks only per-material settings binding, not installed target identity
// or full inheritance flattening; no printer/slicer executable is launched.
func TestRustInstalledPresetSnapshotParity(t *testing.T) {
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	dir := os.Getenv("SLICEBRIDGE_NATIVE_MATERIAL_SNAPSHOT_DIR")
	if binary == "" || dir == "" {
		t.Skip("requires SLICEBRIDGE_RUST_PROBE and SLICEBRIDGE_NATIVE_MATERIAL_SNAPSHOT_DIR")
	}
	entries, err := filepath.Glob(filepath.Join(dir, "*.json"))
	if err != nil || len(entries) == 0 {
		t.Fatalf("no installed material snapshots: %v", err)
	}
	for _, entry := range entries {
		t.Run(filepath.Base(entry), func(t *testing.T) {
			info, err := os.Stat(entry)
			if err != nil {
				t.Fatal(err)
			}
			if !info.Mode().IsRegular() || info.Size() < 10 || info.Size() > 1<<20 {
				t.Fatalf("snapshot must be a regular JSON file <= 1 MB")
			}
			raw, err := os.ReadFile(entry)
			if err != nil {
				t.Fatal(err)
			}
			var snapshot struct {
				Bundle       targetBundle         `json:"bundle"`
				Filaments    []converter.Filament `json:"filaments"`
				ExpectAccept bool                 `json:"expect_accept"`
				Provenance   struct {
					Source string `json:"source"`
					Files  []struct {
						Material string `json:"material"`
						File     string `json:"file"`
						SHA256   string `json:"sha256"`
					} `json:"files"`
				} `json:"provenance"`
			}
			if err := json.Unmarshal(raw, &snapshot); err != nil {
				t.Fatal(err)
			}
			if snapshot.Provenance.Source == "" || len(snapshot.Provenance.Files) == 0 {
				t.Fatal("missing installed-profile provenance")
			}
			for _, origin := range snapshot.Provenance.Files {
				if origin.Material == "" || origin.File == "" || len(origin.SHA256) != 64 {
					t.Fatal("missing origin filename, material or SHA-256")
				}
			}
			// Validate expected success/failure with the Go owner first.
			var goSnapshot struct {
				Bundle    targetBundle         `json:"bundle"`
				Filaments []converter.Filament `json:"filaments"`
			}
			if err := json.Unmarshal(raw, &goSnapshot); err != nil {
				t.Fatal(err)
			}
			goErr := bindNativeFilamentSettings(&goSnapshot.Bundle, goSnapshot.Filaments)
			if (goErr == nil) != snapshot.ExpectAccept {
				t.Fatalf("Go acceptance differs from snapshot expectation: %v", goErr)
			}
			// Independently compare Rust with Go on an untouched copy.
			verifyMaterialMapParity(t, binary, &snapshot.Bundle, snapshot.Filaments)
		})
	}
}
