package bridge

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// Opt-in, deterministic synthetic 3MF stress corpus. This is a converter,
// immutable-source and full ZIP-member parity check, NOT slicer GUI proof or
// a representative benchmark of real user projects.
//
// Generate the corpus with scripts/gen_3mf_stress_corpus.py (outside the repo),
// then set SLICEBRIDGE_STRESS_CORPUS_DIR and SLICEBRIDGE_RUST_PROBE.
func TestRustLargeSyntheticArchiveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	corpus := os.Getenv("SLICEBRIDGE_STRESS_CORPUS_DIR")
	if rust == "" || corpus == "" {
		t.Skip("requires an approved Rust executable and an explicit synthetic 3MF stress corpus directory")
	}
	if info, err := os.Stat(rust); err != nil || !info.Mode().IsRegular() {
		t.Fatal("opt-in Rust binary must exist as a regular file")
	}
	for _, count := range []int{2000, 20000, 60000} {
		t.Run(fmt.Sprintf("%d_tetrahedra", count), func(t *testing.T) {
			name := fmt.Sprintf("synthetic-deflate-%d.3mf", count)
			source := filepath.Join(corpus, name)
			raw, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			if len(raw) == 0 || len(raw) > 250<<20 {
				t.Fatalf("invalid stress 3MF size: %d", len(raw))
			}
			sourceHash := sha256.Sum256(raw)
			dir := t.TempDir()
			baselineOutput := filepath.Join(dir, "go-converted.3mf")
			rustOutput := filepath.Join(dir, "rust-converted.3mf")
			t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "")
			t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", rust)
			start := time.Now()
			baseline, err := ConvertTarget(source, baselineOutput, TargetRequest{Target: "u1"})
			goTime := time.Since(start)
			if err != nil {
				t.Fatalf("real Go production conversion failed: %v", err)
			}
			t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust")
			start = time.Now()
			experimental, err := ConvertTarget(source, rustOutput, TargetRequest{Target: "u1"})
			rustTime := time.Since(start)
			if err != nil {
				t.Fatalf("opt-in Rust production path failed: %v", err)
			}
			if baseline["engine"] != "bl2std" || experimental["engine"] != "rust-3mf" {
				t.Fatalf("wrong engine selectors: Go=%v Rust=%v", baseline["engine"], experimental["engine"])
			}
			for _, key := range []string{"geometry_triangles", "geometry_vertices", "source_filament_slots",
				"target_filament_slots_used", "prime_tower_plates", "removed_slice_cache_entries"} {
				if !reflect.DeepEqual(baseline[key], experimental[key]) {
					t.Fatalf("%s differs: Go=%v Rust=%v", key, baseline[key], experimental[key])
				}
			}
			compareConverted3MFParts(t, baselineOutput, rustOutput)
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(raw, after) || sha256.Sum256(after) != sourceHash {
				t.Fatal("immutable synthetic source changed during Go/Rust stress conversion")
			}
			t.Logf("SYNTHETIC_STRESS_PARITY_PASS tetrahedra=%d source_bytes=%d Go=%s RustRelease=%s source_SHA256=%x",
				count, len(raw), goTime, rustTime, sourceHash)
		})
	}
}
