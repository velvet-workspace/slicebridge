package main

import (
	"bytes"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"testing"
)

// Experimental, host-only native Wails backend acceptance. Both paths use
// App.Convert with the installed manufacturer profiles. No slicer UI, G-code,
// actual printing or user-owned preset file is opened or altered.
func TestRustNativeAppFourTargetFiveMaterialConversion(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires authorized, read-only installed OEM profiles")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	fixturePath := os.Getenv("BRIDGE_QIDI_MIXED_FIVE_SOURCE")
	if binary == "" || fixturePath == "" {
		t.Skip("requires a pinned experimental Rust binary and approved centered five-material 3MF")
	}
	raw, err := os.ReadFile(fixturePath)
	if err != nil {
		t.Fatal(err)
	}
	source := func(dir string) string {
		t.Helper()
		path := filepath.Join(dir, "source.3mf")
		if err := os.WriteFile(path, raw, 0600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	baselineSource := source(t.TempDir())
	rustSource := source(t.TempDir())
	targets := []string{"u1", "qidi", "c5", "cc2"}

	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "")
	baseline := (&App{}).Convert(ConvertRequest{FilePath: baselineSource, Targets: targets})
	if !baseline.OK || len(baseline.Results) != len(targets) || len(baseline.Failures) != 0 {
		t.Fatalf("Go App.Convert baseline failed: %+v", baseline)
	}
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust")
	rustReply := (&App{}).Convert(ConvertRequest{FilePath: rustSource, Targets: targets})
	if !rustReply.OK || len(rustReply.Results) != len(targets) || len(rustReply.Failures) != 0 {
		t.Fatalf("Rust App.Convert failed: %+v", rustReply)
	}
	byTarget := make(map[string]ConvertResult, len(baseline.Results))
	for _, result := range baseline.Results {
		byTarget[result.Target] = result
	}
	for _, result := range rustReply.Results {
		goResult, ok := byTarget[result.Target]
		if !ok || !result.OK || result.Report["engine"] != "rust-3mf" ||
			goResult.Report["engine"] != "bl2std" {
			t.Fatalf("wrong App.Convert engine/target after opt-in: Rust=%+v Go=%+v", result, goResult)
		}
		goParts := appRead3MFParts(t, goResult.OutputPath)
		rustParts := appRead3MFParts(t, result.OutputPath)
		if len(goParts) != len(rustParts) {
			t.Fatalf("%s whole-archive ZIP entry count changed: Go=%d Rust=%d", result.Target, len(goParts), len(rustParts))
		}
		for name, goPart := range goParts {
			rustPart, ok := rustParts[name]
			if !ok {
				t.Fatalf("%s missing Rust 3MF part %q", result.Target, name)
			}
			if bytes.Equal(goPart, rustPart) {
				continue
			}
			if name == "Metadata/project_settings.config" {
				var left, right map[string]any
				if err := json.Unmarshal(goPart, &left); err != nil {
					t.Fatal(err)
				}
				if err := json.Unmarshal(rustPart, &right); err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(left, right) {
					t.Fatalf("%s target project settings differ between Go and Rust", result.Target)
				}
			} else if strings.HasSuffix(name, ".xml") ||
				strings.HasSuffix(name, ".rels") ||
				strings.HasSuffix(name, ".model") ||
				name == "Metadata/model_settings.config" ||
				name == "Metadata/slice_info.config" {
				if !reflect.DeepEqual(nativeRustXMLTokens(t, goPart), nativeRustXMLTokens(t, rustPart)) {
					t.Fatalf("%s target XML part differs from Go at %s", result.Target, name)
				}
			} else {
				t.Fatalf("%s non-XML ZIP part is not byte-identical at %s", result.Target, name)
			}
		}
		if result.Report["source_filament_slots"] != 5 ||
			result.Report["target_filament_slots_used"] != 5 ||
			result.Report["prime_tower_plates"] != goResult.Report["prime_tower_plates"] ||
			result.Report["geometry_triangles"] != goResult.Report["geometry_triangles"] {
			t.Fatalf("%s lost five materials/plate/geometry facts: Rust=%+v Go=%+v",
				result.Target, result.Report, goResult.Report)
		}
		if result.Target == "qidi" &&
			!bytes.Contains(rustParts["3D/3dmodel.model"],
				[]byte("<metadata name=\"Application\">QIDIStudio-01.09.03.51</metadata>")) {
			t.Fatal("Rust App.Convert omitted native QIDI 3MF importer metadata")
		}
		if _, stale := rustParts["Metadata/plate_1.gcode"]; stale {
			t.Fatal("Rust App.Convert left stale machine G-code")
		}
		t.Logf("RUST_NATIVE_APP_FIVE_MATERIAL_PASS target=%s path=%s", result.Target, result.OutputPath)
	}
	for _, src := range []string{baselineSource, rustSource} {
		after, err := os.ReadFile(src)
		if err != nil || !bytes.Equal(raw, after) {
			t.Fatalf("App.Convert modified original source: %s", src)
		}
	}
}

// Native full-member differential matcher: XML element names and attribute
// values must match; only serialization whitespace and attribute ordering may
// differ. Unknown ZIP payloads must match byte-for-byte.
func nativeRustXMLTokens(t *testing.T, data []byte) []string {
	t.Helper()
	d := xml.NewDecoder(bytes.NewReader(data))
	var tokens []string
	for {
		token, err := d.Token()
		if err == io.EOF {
			return tokens
		}
		if err != nil {
			t.Fatalf("invalid Go/Rust converted XML: %v", err)
		}
		switch node := token.(type) {
		case xml.StartElement:
			var attrs []string
			for _, attr := range node.Attr {
				if attr.Name.Space == "xmlns" || (attr.Name.Local == "xmlns" && attr.Name.Space == "") {
					continue
				}
				attrs = append(attrs, fmt.Sprintf("%s/%s=%s", attr.Name.Space, attr.Name.Local, attr.Value))
			}
			sort.Strings(attrs)
			tokens = append(tokens, "START "+node.Name.Space+"/"+node.Name.Local+" "+strings.Join(attrs, "|"))
		case xml.EndElement:
			tokens = append(tokens, "END "+node.Name.Space+"/"+node.Name.Local)
		case xml.CharData:
			if text := strings.TrimSpace(string(node)); text != "" {
				tokens = append(tokens, "TEXT "+text)
			}
		}
	}
}
