package bridge

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustLargeModelArchiveParity generates a private >8 MiB model XML part
// from the pinned source 3MF. This proves that the archive converter's
// streaming paint path does not hit its former small-model allocation limit.
// The 3MF is test-only and never loaded into a slicer or sent to a printer.
func TestRustLargeModelArchiveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	dir := t.TempDir()
	original := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
	source := filepath.Join(dir, "large-model.3mf")
	orig, err := zip.OpenReader(original)
	if err != nil {
		t.Fatal(err)
	}
	defer orig.Close()
	out, err := os.Create(source)
	if err != nil {
		t.Fatal(err)
	}
	zipOut := zip.NewWriter(out)
	enlarged := false
	for _, entry := range orig.File {
		r, err := entry.Open()
		if err != nil {
			t.Fatal(err)
		}
		content, err := io.ReadAll(r)
		r.Close()
		if err != nil {
			t.Fatal(err)
		}
		if entry.Name == "3D/3dmodel.model" {
			needle := []byte("</model>")
			insertion := []byte("<!--" + string(bytes.Repeat([]byte{'x'}, 9<<20)) + "-->")
			index := bytes.LastIndex(content, needle)
			if index < 0 {
				t.Fatal("missing model XML root terminator")
			}
			content = append(append(append([]byte{}, content[:index]...), insertion...), content[index:]...)
			enlarged = true
			if len(content) <= 8<<20 {
				t.Fatal("model did not exceed old 8MB limit")
			}
		}
		header := &zip.FileHeader{Name: entry.Name, Method: entry.Method}
		writer, err := zipOut.CreateHeader(header)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := writer.Write(content); err != nil {
			t.Fatal(err)
		}
	}
	if err := zipOut.Close(); err != nil {
		t.Fatal(err)
	}
	if err := out.Close(); err != nil {
		t.Fatal(err)
	}
	if !enlarged {
		t.Fatal("model member missing")
	}
	inputBytes, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}

	originalParts := inspectZipEntries(t, source)
	var sourceSettings map[string]any
	if err := json.Unmarshal(originalParts["Metadata/project_settings.config"], &sourceSettings); err != nil {
		t.Fatal(err)
	}
	insp, err := converter.Inspect(source)
	if err != nil {
		t.Fatalf("Go failed to inspect expanded source: %v", err)
	}
	slots, mapping, err := sourceSlotPlan(insp.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	profile := printer.Builtin("snapmaker-u1")
	if profile == nil {
		t.Fatal("missing builtin test profile")
	}
	rawGo := filepath.Join(dir, "go-raw.3mf")
	goFinal := filepath.Join(dir, "go-final.3mf")
	rustFinal := filepath.Join(dir, "rust-final.3mf")
	if _, err := converter.Convert(source, rawGo, converter.Options{
		Printer: profile, Slots: slots, Mapping: mapping, Supports: converter.SupportsOff,
	}); err != nil {
		t.Fatalf("Go large-model conversion failed: %v", err)
	}
	removed, err := stripSliceCache(rawGo, goFinal, nil)
	if err != nil {
		t.Fatal(err)
	}
	mapped := map[string]int{}
	for src, dst := range mapping {
		mapped[strconv.Itoa(src)] = dst
	}
	plan := map[string]any{
		"profile": profile, "slots": slots, "slot_count": profile.FilamentSlots,
		"mapping": mapped, "supports": false,
		"source_settings": sourceSettings, "preserved": []string{},
	}
	serialized, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	planPath := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(planPath, serialized, 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	output, commandErr := exec.CommandContext(ctx, rust, "--convert-archive", planPath, source, rustFinal).Output()
	if ctx.Err() != nil {
		t.Fatal(ctx.Err())
	}
	var result struct {
		OK      bool `json:"ok"`
		Removed int  `json:"removed"`
	}
	if err := json.Unmarshal(output, &result); err != nil {
		t.Fatalf("Rust reply invalid: %v %s", err, output)
	}
	if commandErr != nil || !result.OK || result.Removed != removed {
		t.Fatalf("Rust failed to convert large valid model: %v %+v", commandErr, result)
	}
	compareConverted3MFParts(t, goFinal, rustFinal)
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(inputBytes, after) {
		t.Fatal("source changed during conversion")
	}
	if files, _ := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial")); len(files) != 0 {
		t.Fatalf("streaming archive left scratch partial files: %v", files)
	}
	t.Logf("Go/Rust full 3MF ZIP parity on model XML >8MiB; source ZIP=%d bytes; output validated", len(inputBytes))
}
