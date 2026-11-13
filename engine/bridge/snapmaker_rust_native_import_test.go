package bridge

import (
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"testing"
	"time"
)

// TestRustSnapmakerOrcaInstalledOEMMeshImportParity is an opt-in,
// read-only smoke test with the actual installed Snapmaker Orca CLI.
// It tests *mesh information only*: native CLI `--info` is not equivalent
// to a full project preview, material assignment or machine-ready slicing.
//
// Expected scratch inputs are the Go/Rust OEM conversion artifacts generated
// by TestRustFullArchiveInstalledVendorProfileParity on the authorized host.
// Never pass a user project, touch an installed preset or emit G-code.
func TestRustSnapmakerOrcaInstalledOEMMeshImportParity(t *testing.T) {
	slicer := os.Getenv("SLICEBRIDGE_NATIVE_SNAPMAKER_EXE")
	scratch := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR")
	if slicer == "" || scratch == "" {
		t.Skip("set exact installed Snapmaker Orca executable and explicit OEM scratch 3MF directory")
	}
	info, err := os.Stat(slicer)
	if err != nil || !info.Mode().IsRegular() {
		t.Fatalf("Snapmaker Orca CLI not a regular installed file: %q: %v", slicer, err)
	}
	dir := t.TempDir()
	env := append(os.Environ(),
		"APPDATA="+filepath.Join(dir, "appdata"),
		"LOCALAPPDATA="+filepath.Join(dir, "localappdata"),
	)
	for _, folder := range []string{"appdata", "localappdata"} {
		if err := os.MkdirAll(filepath.Join(dir, folder), 0700); err != nil {
			t.Fatal(err)
		}
	}
	for _, model := range []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"} {
		t.Run(model, func(t *testing.T) {
			var imported []map[string]string
			for _, engine := range []string{"go", "rust"} {
				name := fmt.Sprintf("oem-%s-u1-%s", engine, model)
				source := filepath.Join(scratch, name)
				raw, err := os.ReadFile(source)
				if err != nil {
					t.Fatal(err)
				}
				if len(raw) == 0 || len(raw) > 250<<20 {
					t.Fatalf("unsafe scratch 3MF size for %s", name)
				}
				before := sha256.Sum256(raw)
				private := filepath.Join(dir, fmt.Sprintf("native-%s-%s", engine, model))
				if err := os.MkdirAll(private, 0700); err != nil {
					t.Fatal(err)
				}
				ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
				cmd := exec.CommandContext(ctx, slicer, "--info", "--datadir", private, source)
				cmd.Env = env
				output, runErr := cmd.CombinedOutput()
				timedOut := ctx.Err()
				cancel()
				if timedOut != nil {
					t.Fatalf("%s native reader timed out: %v", name, timedOut)
				}
				parsed, parseErr := parsePrusaSlicerInfo(output)
				if parseErr != nil {
					t.Fatalf("%s native mesh metrics malformed: %v", name, parseErr)
				}
				metrics, ok := parsed[name]
				if !ok || metrics["number_of_facets"] == "" || metrics["size_x"] == "" ||
					metrics["size_y"] == "" || metrics["volume"] == "" {
					t.Fatalf("%s no native mesh metrics (command=%v): %s", name, runErr, output)
				}
				// Native --info may log historical plate parsing warnings.
				// A nonzero exit still means the app has not accepted the
				// package as a usable project, even if it reports geometry.
				if runErr != nil {
					t.Fatalf("%s native --info exited with %v despite mesh metrics: %s",
						name, runErr, output)
				}
				after, err := os.ReadFile(source)
				if err != nil || sha256.Sum256(after) != before {
					t.Fatalf("native Snapmaker Orca modified scratch input %s", name)
				}
				imported = append(imported, metrics)
			}
			if !reflect.DeepEqual(imported[0], imported[1]) {
				t.Fatalf("native Snapmaker mesh metrics Go/Rust mismatch: Go=%v Rust=%v", imported[0], imported[1])
			}
			t.Logf("Snapmaker Orca --info accepted Go/Rust OEM %s mesh with equivalent size/facet/volume; native material/tool preview NOT validated", model)
		})
	}
}
