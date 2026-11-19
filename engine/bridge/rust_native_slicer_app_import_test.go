package bridge

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

// Read-only, opt-in *real slicer importer* smoke test of the integrated
// Go-vs-Rust app paths. The PrusaSlicer --info command parses meshes only:
// it does not render the target slicer GUI, slice, emit G-code or print.
// Thus this is additional evidence, NOT a substitute for OEM GUI acceptance.
func TestRustNativeAppReadOnlySlicerMeshImport(t *testing.T) {
	slicer := os.Getenv("SLICEBRIDGE_NATIVE_SLICER_EXE")
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	bundleDir := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	source := os.Getenv("SLICEBRIDGE_RUST_APP_IMPORT_SOURCE")
	if slicer == "" || binary == "" || bundleDir == "" || source == "" {
		t.Skip("needs explicit local PrusaSlicer importer, Rust binary, real OEM bundles and approved 3MF source")
	}
	for _, path := range []string{slicer, binary, source} {
		if stat, err := os.Stat(path); err != nil || !stat.Mode().IsRegular() {
			t.Fatalf("opt-in 3MF/importer path %s is unavailable: %v", path, err)
		}
	}
	before, err := os.ReadFile(source)
	if err != nil || len(before) == 0 || len(before) > 250<<20 {
		t.Fatalf("source unavailable/unsafe size: %v", err)
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	for _, target := range []string{"u1", "qidi"} {
		t.Run(target, func(t *testing.T) {
			dir := t.TempDir()
			profile := filepath.Join(bundleDir, "scratch-"+target+"-0.4-resolved.json")
			goOut := filepath.Join(dir, "go.3mf")
			rustOut := filepath.Join(dir, "rust.3mf")
			goReport, err := ConvertTargetContext(context.Background(), source, goOut, TargetRequest{
				Target: target, Nozzle: "0.4", ProfileFile: profile, Engine: "go",
			})
			if err != nil || goReport["engine"] != "bl2std" {
				t.Fatalf("Go app baseline rejected: %+v %v", goReport, err)
			}
			rustReport, err := ConvertTargetContext(context.Background(), source, rustOut, TargetRequest{
				Target: target, Nozzle: "0.4", ProfileFile: profile, Engine: "rust-native",
			})
			if err != nil || rustReport["engine"] != "rust-native-3mf" || rustReport["planner"] != "rust-native" {
				t.Fatalf("Rust app conversion rejected: %+v %v", rustReport, err)
			}
			infoDir := filepath.Join(dir, "prusa-isolated")
			ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
			defer cancel()
			// One multi-part/plate 3MF can produce many [same-name.3mf]
			// sections in the real PrusaSlicer --info output. Compare every
			// imported part, not a map that assumes one section per file.
			var baseline []byte
			partCount := 0
			for index, path := range []string{source, goOut, rustOut} {
				out, err := exec.CommandContext(ctx, slicer, "--info", "--datadir", infoDir, path).CombinedOutput()
				if ctx.Err() != nil {
					t.Fatalf("native 3MF importer timed out: %v", ctx.Err())
				}
				if err != nil || len(out) > 8<<20 {
					t.Fatalf("native importer refused app-generated 3MF or oversized report: %v (%d bytes)\n%s", err, len(out), out)
				}
				header := []byte("[" + filepath.Base(path) + "]")
				start := bytes.Index(out, header)
				if start < 0 {
					t.Fatalf("native slicer imported no mesh records for %s:\n%s", path, out)
				}
				normalized := bytes.ReplaceAll(out[start:], header, []byte("[MODEL]"))
				if bytes.Count(normalized, []byte("number_of_facets =")) == 0 ||
					bytes.Count(normalized, []byte("volume =")) == 0 {
					t.Fatalf("native slicer missing geometry metrics for %s", path)
				}
				if index == 0 {
					baseline = normalized
					partCount = bytes.Count(normalized, []byte("[MODEL]"))
				} else if !bytes.Equal(baseline, normalized) {
					t.Fatalf("read-only native importer found different part geometry: variant=%s source_parts=%d variant_parts=%d", path, partCount, bytes.Count(normalized, []byte("[MODEL]")))
				}
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(before, after) {
				t.Fatalf("real slicer importer/bridge modified original source: %v", err)
			}
			t.Logf("READ_ONLY_APP_SLICER_IMPORT_PASS target=%s source=%s imported_parts=%d", target, filepath.Base(source), partCount)
		})
	}
}
