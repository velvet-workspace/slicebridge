package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// The Rust-owned native-plan-plus-archive mode must not pad QIDI's
// single-source palette to four fictional tools. Read-only OEM snapshots
// are explicitly supplied, and only disposable 3MF outputs are written.
func TestRustOwnedNativeArchiveSingleMaterialOEMParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if rust == "" || root == "" {
		t.Skip("requires explicit Rust binary and approved installed OEM snapshots")
	}
	source := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
	original, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	filaments, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	if len(filaments.Filaments) != 1 {
		t.Fatalf("expected single source material: %d", len(filaments.Filaments))
	}
	sourceSettings, err := readRustSourceSettings(source)
	if err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{"u1", "qidi"} {
		t.Run(target, func(t *testing.T) {
			snapshot := filepath.Join(root, "scratch-"+target+"-0.4-resolved.json")
			rawBundle, err := os.ReadFile(snapshot)
			if err != nil {
				t.Fatal(err)
			}
			var bundle map[string]any
			if err := json.Unmarshal(rawBundle, &bundle); err != nil {
				t.Fatal(err)
			}
			dir := t.TempDir()
			goOut := filepath.Join(dir, "go.3mf")
			rustOut := filepath.Join(dir, "rust.3mf")
			goReceipt, err := ConvertTarget(source, goOut, TargetRequest{Target: target, ProfileFile: snapshot, Nozzle: "0.4"})
			if err != nil {
				t.Fatalf("Go installed OEM baseline: %v", err)
			}
			if goReceipt["engine"] != "bl2std" {
				t.Fatalf("Go baseline incorrect: %v", goReceipt)
			}
			request, err := json.Marshal(map[string]any{
				"bundle": bundle, "filaments": filaments.Filaments,
				"source_settings": sourceSettings, "requested_nozzle": "0.4",
			})
			if err != nil {
				t.Fatal(err)
			}
			requestPath := filepath.Join(dir, "native-single-request.json")
			if err := os.WriteFile(requestPath, request, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			output, err := exec.CommandContext(ctx, rust, "--convert-native-archive", requestPath, source, rustOut).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var result struct {
				OK           bool
				Experimental bool
				Removed      int
				Error        string
			}
			if parseErr := json.Unmarshal(output, &result); parseErr != nil {
				t.Fatalf("Rust native single receipt invalid: %v; bytes=%s", parseErr, output)
			}
			if err != nil || !result.OK || !result.Experimental {
				t.Fatalf("Rust single material conversion failed: %v %s", err, result.Error)
			}
			compareConverted3MFParts(t, goOut, rustOut)
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(original, after) {
				t.Fatal("source was modified")
			}
			unchanged, err := os.ReadFile(snapshot)
			if err != nil || !bytes.Equal(rawBundle, unchanged) {
				t.Fatal("installed OEM snapshot was modified")
			}
			t.Logf("RUST_NATIVE_SINGLE_OEM_PARITY_PASS target=%s GoSlots=%v removed=%d", target, goReceipt["virtual_filament_slots"], result.Removed)
		})
	}
}
