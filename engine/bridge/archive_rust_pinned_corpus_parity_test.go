package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustPinnedBambuArchiveConversionParity expands the isolated full-Rust
// 3MF conversion test beyond the simple two-triangle synthetic fixture.
// The source corpus is the pinned, checked-in set of Bambu-style 3MF examples;
// these are not customer files and are not real print-quality signoffs.
//
// Optional SLICEBRIDGE_SLICER_OUTPUT_DIR exports source/Go/Rust triplets into
// an explicitly named scratch directory without overwriting existing files.
func TestRustPinnedBambuArchiveConversionParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to compiled Rust binary")
	}
	fixtures, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "bambu-*.3mf"))
	if err != nil || len(fixtures) != 11 {
		t.Fatalf("expected eleven pinned Bambu fixtures: %v count=%d", err, len(fixtures))
	}
	for _, input := range fixtures {
		input := input
		name := filepath.Base(input)
		t.Run(strings.TrimSuffix(name, ".3mf"), func(t *testing.T) {
			scratch := t.TempDir()
			goRaw := filepath.Join(scratch, "go-upstream.3mf")
			goFinal := filepath.Join(scratch, "go-clean.3mf")
			rustFinal := filepath.Join(scratch, "rust-output.3mf")
			sourceBytes, err := os.ReadFile(input)
			if err != nil {
				t.Fatal(err)
			}
			sourceParts := inspectZipEntries(t, input)
			var sourceSettings map[string]any
			if err := json.Unmarshal(sourceParts["Metadata/project_settings.config"], &sourceSettings); err != nil {
				t.Fatalf("source settings not valid JSON: %v", err)
			}
			inspection, err := converter.Inspect(input)
			if err != nil {
				t.Fatalf("Go cannot inspect pinned source: %v", err)
			}
			slots, mapping, err := sourceSlotPlan(inspection.Filaments)
			if err != nil {
				t.Fatalf("Go source filament slot plan invalid: %v", err)
			}
			profile := printer.Builtin("snapmaker-u1")
			if profile == nil {
				t.Fatal("missing builtin test profile")
			}
			if len(slots) > profile.FilamentSlots {
				profile.FilamentSlots = len(slots)
			}
			_, err = converter.Convert(input, goRaw, converter.Options{
				Printer: profile, Slots: slots, Mapping: mapping,
				Supports: converter.SupportsOff,
			})
			if err != nil {
				t.Fatalf("Go conversion declined pinned source: %v", err)
			}
			goRemoved, err := stripSliceCache(goRaw, goFinal, nil)
			if err != nil {
				t.Fatal(err)
			}
			jsonMapping := make(map[string]int, len(mapping))
			for src, dest := range mapping {
				jsonMapping[strconv.Itoa(src)] = dest
			}
			plan := map[string]any{
				"profile":         profile,
				"slots":           slots,
				"slot_count":      profile.FilamentSlots,
				"supports":        false,
				"source_settings": sourceSettings,
				"mapping":         jsonMapping,
				"preserved":       []string{},
			}
			encoded, err := json.Marshal(plan)
			if err != nil {
				t.Fatal(err)
			}
			planPath := filepath.Join(scratch, "plan.json")
			if err := os.WriteFile(planPath, encoded, 0600); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
			defer cancel()
			stdout, cmdErr := exec.CommandContext(ctx, rust,
				"--convert-archive", planPath, input, rustFinal).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var reply struct {
				OK      bool `json:"ok"`
				Removed int  `json:"removed"`
			}
			if err := json.Unmarshal(stdout, &reply); err != nil {
				t.Fatalf("Rust CLI returned invalid JSON: %v %q command=%v", err, stdout, cmdErr)
			}
			if cmdErr != nil || !reply.OK || reply.Removed != goRemoved {
				t.Fatalf("Rust pinned-source conversion failed, Go caches=%d Rust reply=%+v cmdErr=%v raw=%s", goRemoved, reply, cmdErr, stdout)
			}
			compareConverted3MFParts(t, goFinal, rustFinal)
			sourceAfter, err := os.ReadFile(input)
			if err != nil || !bytes.Equal(sourceBytes, sourceAfter) {
				t.Fatalf("pinned source modified unexpectedly: %v", err)
			}
			if targetDir := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR"); targetDir != "" {
				if info, err := os.Stat(targetDir); err != nil || !info.IsDir() {
					t.Fatalf("scratch output dir %s unavailable: %v", targetDir, err)
				}
				for _, item := range [][2]string{
					{input, "pinned-source-" + name},
					{goFinal, "pinned-go-" + name},
					{rustFinal, "pinned-rust-" + name},
				} {
					exportSlicerFixtureNoOverwrite(t, item[0], filepath.Join(targetDir, item[1]))
				}
			}
			t.Logf("pinned input %s: %d declared source filaments, %d target slots; full ZIP parity passed", name, len(slots), profile.FilamentSlots)
		})
	}
}
