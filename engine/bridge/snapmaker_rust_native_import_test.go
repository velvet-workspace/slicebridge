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

// TestRustSnapmakerOrcaInstalledOEMMeshImportParity is the original opt-in
// geometry-only native CLI gate for machine-only OEM Go/Rust 3MF packages.
// It is not a material/tool preview or proof of a print-ready project.
func TestRustSnapmakerOrcaInstalledOEMMeshImportParity(t *testing.T) {
	testNativeSnapmakerOrcaOEMGeometry(t, "SLICEBRIDGE_SLICER_OUTPUT_DIR", "oem", []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"})
}

// TestRustSnapmakerOrcaMaterialBoundOEMMeshImportParity adds a second native
// gate over Go's *production material-bound* OEM output and the semantically
// equivalent Rust ZIP. Unlike the machine-only files above, these projects
// include the installed target's material profile IDs and temperature/flow
// fields. The native CLI still validates mesh import only; material/tool
// assignment requires a separate GUI preview gate.
func TestRustSnapmakerOrcaMaterialBoundOEMMeshImportParity(t *testing.T) {
	testNativeSnapmakerOrcaOEMGeometry(t, "SLICEBRIDGE_BOUND_OEM_OUTPUT_DIR", "bound", []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"})
}

// TestRustSnapmakerOrcaVerifiedPETGMixedLeafMeshImportParity is an opt-in
// native CLI geometry smoke over the new material-bound U1 0.4mm PETG
// manufacturer-leaf output. It still does not prove slicer tool assignments,
// firmware commands or printability: mesh-only native --info validation.
func TestRustSnapmakerOrcaVerifiedPETGMixedLeafMeshImportParity(t *testing.T) {
	testNativeSnapmakerOrcaOEMGeometry(t, "SLICEBRIDGE_VERIFIED_U1_PETG_OUTPUT_DIR", "bound", []string{"mixed-five.3mf"})
}

func testNativeSnapmakerOrcaOEMGeometry(t *testing.T, directoryEnv, prefix string, models []string) {
	t.Helper()
	slicer := os.Getenv("SLICEBRIDGE_NATIVE_SNAPMAKER_EXE")
	scratch := os.Getenv(directoryEnv)
	if slicer == "" || scratch == "" {
		t.Skipf("set installed Snapmaker Orca executable and %s to generated scratch 3MF outputs", directoryEnv)
	}
	info, err := os.Stat(slicer)
	if err != nil || !info.Mode().IsRegular() {
		t.Fatalf("Snapmaker Orca CLI not a regular installed file: %q: %v", slicer, err)
	}
	info, err = os.Stat(scratch)
	if err != nil || !info.IsDir() {
		t.Fatalf("%s must point at an existing scratch output folder: %v", directoryEnv, err)
	}
	isolated := t.TempDir()
	env := append(os.Environ(),
		"APPDATA="+filepath.Join(isolated, "appdata"),
		"LOCALAPPDATA="+filepath.Join(isolated, "localappdata"),
	)
	for _, folder := range []string{"appdata", "localappdata"} {
		if err := os.MkdirAll(filepath.Join(isolated, folder), 0700); err != nil {
			t.Fatal(err)
		}
	}
	for _, model := range models {
		t.Run(model, func(t *testing.T) {
			var imported []map[string]string
			for _, engine := range []string{"go", "rust"} {
				name := fmt.Sprintf("%s-%s-u1-%s", prefix, engine, model)
				source := filepath.Join(scratch, name)
				raw, err := os.ReadFile(source)
				if err != nil {
					t.Fatal(err)
				}
				if len(raw) == 0 || len(raw) > 250<<20 {
					t.Fatalf("unsafe scratch 3MF size for %s", name)
				}
				before := sha256.Sum256(raw)
				private := filepath.Join(isolated, fmt.Sprintf("native-%s-%s", engine, model))
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
				// The CLI may emit project plate/XML parser warnings while
				// still returning successful mesh information. Even exit 0
				// is not proof of material/tool-preview readiness.
				if runErr != nil {
					t.Fatalf("%s native --info failed despite mesh metrics: %v %s", name, runErr, output)
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
			t.Logf("Snapmaker Orca %s --info matched Go/Rust %s geometry; native tool/material preview NOT validated", prefix, model)
		})
	}
}
