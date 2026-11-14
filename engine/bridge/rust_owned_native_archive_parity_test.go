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

// Opt-in cross-language acceptance of Rust-owned OEM material/slot planning
// PLUS Rust's full 3MF writer, against the installed-profile Go production
// result. The test supplies a snapshot only; no slicer GUI/print operations.
// Rust independently selects safe source process intent when "preserved"
// is omitted. It does not invoke Go's buildPlan or preservedProfile for its
// production-sized plan; Go is used only as the parity baseline.
func TestRustOwnedNativeArchiveInstalledOEMParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	source := os.Getenv("SLICEBRIDGE_MIXED_FIVE_SOURCE")
	if rust == "" || root == "" || source == "" {
		t.Skip("requires approved Rust binary, installed OEM snapshots, and mixed-five 3MF source")
	}
	original, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	info, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	sourceSettings, err := readRustSourceSettings(source)
	if err != nil {
		t.Fatal(err)
	}
	if len(info.Filaments) != 5 {
		t.Fatalf("expected five source virtual filaments, got %d", len(info.Filaments))
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
			goOutput := filepath.Join(dir, "go.3mf")
			rustOutput := filepath.Join(dir, "rust.3mf")
			targetRequest := TargetRequest{Target: target, ProfileFile: snapshot, Nozzle: "0.4"}
			goReceipt, err := ConvertTarget(source, goOutput, targetRequest)
			if err != nil {
				t.Fatalf("Go OEM baseline conversion: %v", err)
			}
			if goReceipt["engine"] != "bl2std" {
				t.Fatalf("Go baseline not Go: %v", goReceipt["engine"])
			}
			// No Go plan or Go-preserved list is passed to the experimental
			// Rust-native converter. The request contains only verified
			// OEM data, source filament identity and source settings.
			request := map[string]any{
				"bundle":          bundle,
				"filaments":       info.Filaments,
				"source_settings": sourceSettings,
			}
			payload, err := json.Marshal(request)
			if err != nil {
				t.Fatal(err)
			}
			if len(payload) > 8<<20 {
				t.Fatal("native request exceeds Rust JSON limit")
			}
			requestFile := filepath.Join(dir, "verified-oem-request.json")
			if err := os.WriteFile(requestFile, payload, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
			defer cancel()
			command := exec.CommandContext(ctx, rust, "--convert-native-archive", requestFile, source, rustOutput)
			output, runErr := command.Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var response struct {
				OK           bool   `json:"ok"`
				Experimental bool   `json:"experimental"`
				Removed      int    `json:"removed"`
				Error        string `json:"error"`
			}
			if err := json.Unmarshal(output, &response); err != nil {
				t.Fatalf("Rust native archive receipt invalid: %v", err)
			}
			if runErr != nil || !response.OK || !response.Experimental || response.Removed < 0 {
				t.Fatalf("Rust native archive refused verified input: status=%v error=%s", runErr, response.Error)
			}
			compareConverted3MFParts(t, goOutput, rustOutput)
			after, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(original, after) {
				t.Fatal("Rust or Go changed source")
			}
			updatedBundle, err := os.ReadFile(snapshot)
			if err != nil || !bytes.Equal(rawBundle, updatedBundle) {
				t.Fatal("verified installed OEM snapshot changed")
			}
			leftovers, err := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial"))
			if err != nil {
				t.Fatal(err)
			}
			if len(leftovers) != 0 {
				t.Fatalf("Rust native archive left temporary stages: %v", leftovers)
			}
			t.Logf("RUST_OWNED_NATIVE_ARCHIVE_PARITY_PASS target=%s source_filaments=5 archive_parts=full removed_caches=%d", target, response.Removed)
		})
	}
}
