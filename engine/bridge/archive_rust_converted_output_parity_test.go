package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// TestRustSanitizeConverted3MFParity exercises actual Go 3MF conversion output
// before cache cleanup, then compares its Rust sanitization against BOTH the
// standalone Go sanitizer and the full production Go conversion. It does not
// print, slice, modify the running app, or publish user files.
func TestRustSanitizeConverted3MFParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe binary")
	}
	for _, filaments := range []int{1, 4, 6} {
		t.Run(strconv.Itoa(filaments)+"_virtual_slots", func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			fixture(t, source, filaments)

			// Use the production plan builder, target resolver and profile
			// preservation policy instead of inventing a Rust printer preset.
			plan, err := buildPlan(source, filepath.Join(dir, "planned.3mf"),
				func(src NozzleInfo) (*ResolvedTargetProfile, error) {
					return resolveBuiltinTarget("u1", "", "", src.Nozzle)
				})
			if err != nil {
				t.Fatal(err)
			}
			if len(plan.slots) > plan.base.FilamentSlots {
				plan.base.FilamentSlots = len(plan.slots)
			}
			merged, preserved, err := preservedProfile(source, plan.base)
			if err != nil {
				t.Fatal(err)
			}

			rawPath := filepath.Join(dir, "raw-upstream.3mf")
			_, err = converter.Convert(source, rawPath, converter.Options{
				Printer: merged, Slots: plan.slots, Mapping: plan.mapping,
				Supports: converter.SupportsAuto,
			})
			if err != nil {
				t.Fatalf("actual Go upstream 3MF converter failed: %v", err)
			}
			goClean := filepath.Join(dir, "go-sanitized.3mf")
			rustClean := filepath.Join(dir, "rust-sanitized.3mf")
			production := filepath.Join(dir, "go-production.3mf")
			goRemoved, err := stripSliceCache(rawPath, goClean, preserved)
			if err != nil {
				t.Fatal(err)
			}

			args := []string{"--strip-cache"}
			for _, key := range preserved {
				args = append(args, "--preserve", key)
			}
			args = append(args, rawPath, rustClean)
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			output, commandErr := exec.CommandContext(ctx, rust, args...).Output()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var result struct {
				OK      bool   `json:"ok"`
				Removed int    `json:"removed"`
				Error   string `json:"error"`
			}
			if err := json.Unmarshal(output, &result); err != nil {
				t.Fatalf("invalid Rust JSON response: %v (%s)", err, output)
			}
			if commandErr != nil || !result.OK || result.Removed != goRemoved {
				t.Fatalf("Rust cleanup differs: Go removed=%d Rust reply=%+v error=%v", goRemoved, result, commandErr)
			}

			if _, err := Convert(source, production, "u1"); err != nil {
				t.Fatalf("production Go conversion failed: %v", err)
			}
			compare3MFParts(t, goClean, rustClean)
			compare3MFParts(t, production, rustClean)

			for _, target := range []string{rustClean, goClean, production} {
				facts, err := inspectPackage(target)
				if err != nil {
					t.Fatalf("Go rejects converted file %s: %v", filepath.Base(target), err)
				}
				if facts.sliceCaches != 0 {
					t.Fatalf("%s still contains %d cached G-codes", target, facts.sliceCaches)
				}
				if facts.vertices != plan.before.vertices || facts.triangles != plan.before.triangles {
					t.Fatalf("%s changed mesh geometry counts", target)
				}
				if facts.printerModel != "Snapmaker U1" {
					t.Fatalf("%s has incorrect target printer model %q", target, facts.printerModel)
				}
			}
		})
	}
}

// Compare package meaning, not ZIP compression/container byte layout.
// Unmodified payloads must match byte-for-byte; JSON settings are semantic;
// OPC XML writers may serialize identical namespaces/attributes differently.
func compare3MFParts(t *testing.T, expected, actual string) {
	t.Helper()
	wantParts := inspectZipEntries(t, expected)
	gotParts := inspectZipEntries(t, actual)
	if len(wantParts) != len(gotParts) {
		t.Fatalf("3MF package count differs: %s=%d vs %s=%d",
			filepath.Base(expected), len(wantParts), filepath.Base(actual), len(gotParts))
	}
	for name, data := range wantParts {
		content, ok := gotParts[name]
		if !ok {
			t.Fatalf("%s missing 3MF part %s", filepath.Base(actual), name)
		}
		switch {
		case name == "Metadata/project_settings.config":
			var a, b any
			if err := json.Unmarshal(data, &a); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(content, &b); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(a, b) {
				t.Fatalf("3MF project setting content differs in %s", name)
			}
		case name == "[Content_Types].xml" || strings.HasSuffix(name, ".rels"):
			if !reflect.DeepEqual(semanticXMLTokens(t, data), semanticXMLTokens(t, content)) {
				t.Fatalf("3MF OPC XML differs in %s", name)
			}
		default:
			if !bytes.Equal(data, content) {
				t.Fatalf("3MF payload differs in %s", name)
			}
		}
	}
}
