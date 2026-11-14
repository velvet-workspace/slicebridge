package bridge

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
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
			out, err := exec.CommandContext(ctx, slicer, "--info", "--datadir", infoDir, source, goOut, rustOut).CombinedOutput()
			if ctx.Err() != nil {
				t.Fatalf("native 3MF importer timed out: %v", ctx.Err())
			}
			if err != nil {
				t.Fatalf("native importer refused app-generated 3MF: %v\n%s", err, out)
			}
			records, err := parsePrusaSlicerInfo(out)
			if err != nil || len(records) != 3 {
				t.Fatalf("unexpected native importer receipt: %v (%d)\n%s", err, len(records), out)
			}
			originalName := filepath.Base(source)
			srcFacts, goFacts, rustFacts := records[originalName], records["go.3mf"], records["rust.3mf"]
			if !reflect.DeepEqual(srcFacts, goFacts) || !reflect.DeepEqual(goFacts, rustFacts) {
				t.Fatalf("app-generated 3MF mesh import mismatch: source=%v Go=%v Rust=%v", srcFacts, goFacts, rustFacts)
			}
			for _, key := range []string{"number_of_facets", "number_of_parts", "size_x", "size_y", "size_z", "volume"} {
				if srcFacts[key] == "" {
					t.Fatalf("native slicer imported without %s: %v", key, srcFacts)
				}
			}
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(before, after) {
				t.Fatalf("real slicer importer/bridge modified original source: %v", err)
			}
			t.Logf("READ_ONLY_APP_SLICER_IMPORT_PASS target=%s mesh=%s triangles=%s", target, originalName, srcFacts["number_of_facets"])
		})
	}
}
