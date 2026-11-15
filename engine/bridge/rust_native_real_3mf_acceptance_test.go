package bridge

import (
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// This is an explicit, read-only, *real* Velvet Factory project-corpus gate.
// Local user-owned 3MF projects are not in Git and never copied into commits;
// temporary captured sources and Go/Rust outputs are removed by Go testing.
// No slicer is launched, no slicing/G-code/print is performed.
func TestRustNativeAuthorizedRealProjectsArchiveParity(t *testing.T) {
	corpusDir := os.Getenv("SLICEBRIDGE_REAL_3MF_CORPUS_DIR")
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	oem := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if corpusDir == "" || binary == "" || oem == "" {
		t.Skip("requires explicitly approved 3MF work corpus, Rust binary and read-only installed OEM snapshots")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	cases := []struct {
		name, filename           string
		expectedMinimumMaterials int
	}{
		{"multicolor-five", "Mini_Chick_Multicolor_5color_v02_bambu.3mf", 5},
		{"supports-petg", "Foldable_Hanger+Rounded+105.3mf", 1},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			originalPath := filepath.Join(corpusDir, tc.filename)
			stat, err := os.Lstat(originalPath)
			if err != nil || !stat.Mode().IsRegular() || stat.Size() < 1000 || stat.Size() > 250<<20 {
				t.Skipf("authorized original unavailable or outside 1 KiB-250 MiB: %s (%v)", tc.filename, err)
			}
			raw, err := os.ReadFile(originalPath)
			if err != nil {
				t.Fatal(err)
			}
			originalHash := sha256.Sum256(raw)
			dir := t.TempDir()
			staged := filepath.Join(dir, "staged.3mf")
			if err := os.WriteFile(staged, raw, 0600); err != nil {
				t.Fatal(err)
			}
			raw = nil
			before, err := inspectPackage(staged)
			if err != nil {
				t.Fatalf("real source invalid 3MF before conversion: %v", err)
			}
			if before.triangles < 1 {
				t.Fatal("real source has no model triangles")
			}
			for _, target := range []string{"u1", "qidi"} {
				t.Run(target, func(t *testing.T) {
					bundle := filepath.Join(oem, fmt.Sprintf("scratch-%s-0.4-resolved.json", target))
					if s, err := os.Stat(bundle); err != nil || !s.Mode().IsRegular() {
						t.Fatalf("verified installed OEM bundle missing for %s: %v", target, err)
					}
					outdir := t.TempDir()
					goPath := filepath.Join(outdir, "go.3mf")
					rustPath := filepath.Join(outdir, "rust.3mf")
					req := TargetRequest{Target: target, Nozzle: "0.4", ProfileFile: bundle, Engine: "go"}
					start := time.Now()
					ctx, cancel := context.WithTimeout(context.Background(), 180*time.Second)
					defer cancel()
					goReceipt, err := ConvertTargetContext(ctx, staged, goPath, req)
					if err != nil || goReceipt["engine"] != "bl2std" {
						t.Fatalf("Go app conversion refused authorized real 3MF: %v report=%v", err, goReceipt)
					}
					goElapsed := time.Since(start)
					req.Engine = "rust-native"
					start = time.Now()
					rustReceipt, err := ConvertTargetContext(ctx, staged, rustPath, req)
					if err != nil || rustReceipt["engine"] != "rust-native-3mf" || rustReceipt["planner"] != "rust-native" {
						t.Fatalf("Rust app conversion refused authorized real 3MF: %v report=%v", err, rustReceipt)
					}
					rustElapsed := time.Since(start)
					for _, key := range []string{
						"geometry_vertices", "geometry_triangles", "source_filament_slots",
						"target_nozzle", "target_filament_slots_used", "physical_toolheads",
						"removed_slice_cache_entries", "source_to_target_slot_map",
					} {
						if !reflect.DeepEqual(goReceipt[key], rustReceipt[key]) {
							t.Fatalf("%s differs Go=%v Rust=%v", key, goReceipt[key], rustReceipt[key])
						}
					}
					if count, ok := rustReceipt["source_filament_slots"].(int); ok && count < tc.expectedMinimumMaterials {
						t.Fatalf("real multicolor source unexpectedly lost material slots: %d", count)
					}
					compareConverted3MFParts(t, goPath, rustPath)
					output, err := inspectPackage(rustPath)
					if err != nil || output.triangles != before.triangles || output.vertices != before.vertices || output.sliceCaches != 0 {
						t.Fatalf("real Rust 3MF changed geometry or retained slicing caches: %+v err=%v", output, err)
					}
					t.Logf("REAL_3MF_GO_RUST_PARITY_PASS project=%s target=%s source_bytes=%d triangles=%d source_materials=%v go_ms=%.1f rust_ms=%.1f",
						tc.name, target, stat.Size(), before.triangles, rustReceipt["source_filament_slots"],
						float64(goElapsed.Microseconds())/1000, float64(rustElapsed.Microseconds())/1000)
				})
			}
			after, err := os.ReadFile(originalPath)
			if err != nil || sha256.Sum256(after) != originalHash {
				t.Fatalf("source model changed during real corpus tests: %v", err)
			}
		})
	}
}
