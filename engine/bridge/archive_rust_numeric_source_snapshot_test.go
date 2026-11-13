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

// rewriteNumericTestSource writes only to a new test directory. It uses actual
// raw JSON numeric tokens, not a pre-normalized Go float64 representation.
func rewriteNumericTestSource(t *testing.T, original, destination, numericJSON string) {
	t.Helper()
	source, err := zip.OpenReader(original)
	if err != nil {
		t.Fatal(err)
	}
	defer source.Close()
	output, err := os.OpenFile(destination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	writer := zip.NewWriter(output)
	found := false
	for _, entry := range source.File {
		input, err := entry.Open()
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(input)
		input.Close()
		if err != nil {
			t.Fatal(err)
		}
		if entry.Name == "Metadata/project_settings.config" {
			var settings map[string]json.RawMessage
			if err := json.Unmarshal(body, &settings); err != nil {
				t.Fatal(err)
			}
			settings["numeric_snapshot_qa"] = json.RawMessage(numericJSON)
			body, err = json.Marshal(settings)
			if err != nil {
				t.Fatal(err)
			}
			found = true
		}
		member, err := writer.CreateHeader(&zip.FileHeader{Name: entry.Name, Method: entry.Method})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := member.Write(body); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := output.Close(); err != nil {
		t.Fatal(err)
	}
	if !found {
		t.Fatal("source fixture had no project settings")
	}
}

func TestRustNumericSourceSnapshotFullArchiveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	dir := t.TempDir()
	base := filepath.Join(dir, "base.3mf")
	source := filepath.Join(dir, "source.3mf")
	fixture(t, base, 1)
	rewriteNumericTestSource(t, base, source, `[1.0,1e0,-0.0,{"nested":2.50}]`)
	before, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}

	var sourceSettings map[string]any
	if err := json.Unmarshal(inspectZipEntries(t, source)["Metadata/project_settings.config"], &sourceSettings); err != nil {
		t.Fatal(err)
	}
	inspection, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	slots, mapping, err := sourceSlotPlan(inspection.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	profile := printer.Builtin("snapmaker-u1")
	if profile == nil {
		t.Fatal("missing test target profile")
	}
	goRaw := filepath.Join(dir, "go-raw.3mf")
	goFinal := filepath.Join(dir, "go-final.3mf")
	rustFinal := filepath.Join(dir, "rust-final.3mf")
	if _, err := converter.Convert(source, goRaw, converter.Options{
		Printer: profile, Slots: slots, Mapping: mapping, Supports: converter.SupportsOff,
	}); err != nil {
		t.Fatal(err)
	}
	goRemoved, err := stripSliceCache(goRaw, goFinal, nil)
	if err != nil {
		t.Fatal(err)
	}
	mappingJSON := map[string]int{}
	for src, dst := range mapping {
		mappingJSON[strconv.Itoa(src)] = dst
	}
	plan := map[string]any{
		"profile": profile, "slots": slots, "slot_count": profile.FilamentSlots,
		"supports": false, "source_settings": sourceSettings,
		"mapping": mappingJSON, "preserved": []string{},
	}
	planBytes, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	planFile := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(planFile, planBytes, 0600); err != nil {
		t.Fatal(err)
	}

	run := func(planPath, destination string) ([]byte, error) {
		t.Helper()
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		output, err := exec.CommandContext(ctx, rust, "--convert-archive", planPath, source, destination).Output()
		if ctx.Err() != nil {
			t.Fatal(ctx.Err())
		}
		return output, err
	}
	output, runErr := run(planFile, rustFinal)
	var reply struct {
		OK      bool `json:"ok"`
		Removed int  `json:"removed"`
	}
	if err := json.Unmarshal(output, &reply); err != nil {
		t.Fatalf("bad Rust JSON response: %v %s", err, output)
	}
	if runErr != nil || !reply.OK || reply.Removed != goRemoved {
		t.Fatalf("Go-compatible raw numeric literals refused: %v %+v", runErr, reply)
	}
	compareConverted3MFParts(t, goFinal, rustFinal)
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("numeric source changed")
	}
	t.Log("Go/Rust full 3MF archive parity passed with source 1.0/1e0/-0.0/2.50 versus Go-normalized request")

	// Changing the material-facing numeric snapshot is never treated as a
	// harmless formatting difference. Reject and publish nothing.
	stale := map[string]any{}
	for key, value := range sourceSettings {
		stale[key] = value
	}
	stale["numeric_snapshot_qa"] = []any{1.5, 1.0, 0.0, map[string]any{"nested": 2.5}}
	plan["source_settings"] = stale
	bad, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	badPlan := filepath.Join(dir, "stale.json")
	if err := os.WriteFile(badPlan, bad, 0600); err != nil {
		t.Fatal(err)
	}
	badOut := filepath.Join(dir, "must-not-exist.3mf")
	rejected, runErr := run(badPlan, badOut)
	if runErr == nil {
		t.Fatalf("stale numeric snapshot accepted: %s", rejected)
	}
	if _, err := os.Stat(badOut); !os.IsNotExist(err) {
		t.Fatal("stale numeric snapshot published output")
	}
}

func TestRustNumericSnapshotRejectsAmbiguousHugeIntegers(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	dir := t.TempDir()
	original := filepath.Join(dir, "base.3mf")
	source := filepath.Join(dir, "unsafe-large-int.3mf")
	fixture(t, original, 1)
	rewriteNumericTestSource(t, original, source, `[9007199254740993]`)
	var settings map[string]any
	if err := json.Unmarshal(inspectZipEntries(t, source)["Metadata/project_settings.config"], &settings); err != nil {
		t.Fatal(err)
	}
	// Go normalizes this integer into a rounded float64. Rust must fail
	// closed instead of silently accepting a distinct source setting.
	inspection, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	slots, mapping, err := sourceSlotPlan(inspection.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	mapJSON := map[string]int{}
	for from, to := range mapping {
		mapJSON[strconv.Itoa(from)] = to
	}
	plan := map[string]any{
		"profile": printer.Builtin("snapmaker-u1"),
		"slots":   slots, "slot_count": 4, "supports": false,
		"source_settings": settings, "mapping": mapJSON,
	}
	body, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	p := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(p, body, 0600); err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(dir, "must-not-exist.3mf")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	reply, runErr := exec.CommandContext(ctx, rust, "--convert-archive", p, source, dst).Output()
	if ctx.Err() != nil {
		t.Fatal(ctx.Err())
	}
	if runErr == nil {
		t.Fatalf("ambiguous 2^53+1 numeric snapshot accepted: %s", reply)
	}
	if _, err := os.Stat(dst); !os.IsNotExist(err) {
		t.Fatal("unsafe integer created output")
	}
}
