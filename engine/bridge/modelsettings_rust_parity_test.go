package bridge

import (
	"archive/zip"
	"bytes"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// Compare standalone Rust model_settings XML rewriting against actual Go
// converted 3MF outputs. This exercises real Go converter internals rather
// than duplicating their expected output algorithm in Go test code.
func TestRustModelSettingsConverted3MFParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	const sourceXML = `<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="1"><metadata key="extruder" value="1"/>
     <metadata key="extruder" value="1"/>
     <metadata key="extruder" value="not-a-number"/></object>
  <plate><metadata key="filament_maps" value="1 1 1 1"/></plate>
  <metadata key="notes" value="A&amp;B"/>
</config>`

	for _, sourceCount := range []int{1, 4, 6} {
		t.Run(strconv.Itoa(sourceCount)+"_source_filaments", func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			production := filepath.Join(dir, "converted-go.3mf")
			original := fixture(t, source, sourceCount)
			replaceZIPPart(t, source, "Metadata/model_settings.config", []byte(sourceXML))
			modified, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			if bytes.Equal(original, modified) {
				t.Fatal("fixture setup did not replace model settings")
			}

			// The production converter uses its own material mapping and
			// hardware base slots. Read the source via the same inspect/plan
			// functions rather than assuming a mapping.
			inspection, err := converter.Inspect(source)
			if err != nil {
				t.Fatal(err)
			}
			_, mapping, err := sourceSlotPlan(inspection.Filaments)
			if err != nil {
				t.Fatal(err)
			}
			strMap := map[string]int{}
			for src, target := range mapping {
				strMap[strconv.Itoa(src)] = target
			}
			baseSlots := sourceCount
			if baseSlots < 4 {
				baseSlots = 4 // legacy Snapmaker U1 physical profile
			}
			reply, rustErr := runRustMaterialCommand(t, rust, "--rewrite-model-settings", map[string]any{
				"mapping":            strMap,
				"slots":              baseSlots,
				"model_settings_xml": sourceXML,
			})
			if rustErr != nil || reply["ok"] != true {
				t.Fatalf("Rust model metadata rewrite failed: %v %v", rustErr, reply)
			}
			rustXML, ok := reply["model_settings_xml"].(string)
			if !ok {
				t.Fatalf("Rust response missing XML: %v", reply)
			}
			if _, err := Convert(source, production, "u1"); err != nil {
				t.Fatalf("actual Go converter failed: %v", err)
			}
			parts := inspectZipEntries(t, production)
			goXML := parts["Metadata/model_settings.config"]
			left := semanticXMLTokens(t, goXML)
			right := semanticXMLTokens(t, []byte(rustXML))
			if !reflect.DeepEqual(left, right) {
				t.Fatalf("Go/Rust converted model metadata differs\nGo: %s\nRust: %s", goXML, rustXML)
			}
			srcNow, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(srcNow, modified) {
				t.Fatal("source 3MF was unexpectedly modified")
			}
		})
	}
}

func replaceZIPPart(t *testing.T, filename, member string, replacement []byte) {
	t.Helper()
	source, err := zip.OpenReader(filename)
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	writer := zip.NewWriter(&output)
	found := false
	for _, entry := range source.File {
		reader, err := entry.Open()
		if err != nil {
			t.Fatal(err)
		}
		content, err := io.ReadAll(io.LimitReader(reader, 8<<20))
		reader.Close()
		if err != nil {
			t.Fatal(err)
		}
		if entry.Name == member {
			content = replacement
			found = true
		}
		dst, err := writer.Create(entry.Name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := dst.Write(content); err != nil {
			t.Fatal(err)
		}
	}
	source.Close()
	if !found {
		t.Fatalf("member %q absent from synthetic ZIP fixture", member)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, output.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
}

func TestRustModelSettingsNonIdentityMappingParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to compiled Rust probe")
	}
	dir := t.TempDir()
	input := filepath.Join(dir, "source.3mf")
	output := filepath.Join(dir, "converted.3mf")
	fixture(t, input, 2)
	const modelXML = "<config><object id=\"1\"><metadata key=\"extruder\" value=\"2\"/><metadata key=\"extruder\" value=\"1\"/></object><plate><metadata key=\"filament_maps\" value=\"2 1 2 1\"/></plate></config>"
	replaceZIPPart(t, input, "Metadata/model_settings.config", []byte(modelXML))

	mapping := map[int]int{1: 2, 2: 1}
	options := converter.Options{
		Printer: printer.Builtin("snapmaker-u1"),
		Slots: []converter.Slot{
			{Color: "#FF0000", Type: "PLA"},
			{Color: "#00FF00", Type: "PLA"},
		},
		Mapping:  mapping,
		Supports: converter.SupportsOff,
	}
	if _, err := converter.Convert(input, output, options); err != nil {
		t.Fatalf("actual Go conversion rejected valid explicit mapping: %v", err)
	}
	contents := inspectZipEntries(t, output)
	expected := contents["Metadata/model_settings.config"]
	reply, err := runRustMaterialCommand(t, rust, "--rewrite-model-settings", map[string]any{
		"model_settings_xml": modelXML,
		"slots":              4,
		"mapping":            map[string]int{"1": 2, "2": 1},
	})
	if err != nil || reply["ok"] != true {
		t.Fatalf("Rust declined explicit mapping: %v %v", err, reply)
	}
	actual, ok := reply["model_settings_xml"].(string)
	if !ok {
		t.Fatal("Rust response omitted XML")
	}
	if !reflect.DeepEqual(semanticXMLTokens(t, expected), semanticXMLTokens(t, []byte(actual))) {
		t.Fatalf("Go/Rust explicit mapped model metadata differs: Go=%s Rust=%s", expected, actual)
	}
}
