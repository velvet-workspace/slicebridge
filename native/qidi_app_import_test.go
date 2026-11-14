package main

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// Host-only app entrypoint QA. Exercises App.Convert, the same backend
// invoked by the Wails frontend, with installed QIDIStudio presets.
// It neither opens a slicer nor slices/prints or edits installed profiles.
func TestQIDIAppFiveFilamentImportOnInstalledHost(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires approved local QIDIStudio installation")
	}
	source := loadFixture(t, "bambu-five-colors.3mf")
	if custom := os.Getenv("BRIDGE_QIDI_MIXED_FIVE_SOURCE"); custom != "" {
		data, err := os.ReadFile(custom)
		if err != nil {
			t.Fatal(err)
		}
		source = filepath.Join(t.TempDir(), "source.3mf")
		if err := os.WriteFile(source, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	original, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	sourceParts := appRead3MFParts(t, source)
	var expected map[string]any
	if err := json.Unmarshal(sourceParts["Metadata/project_settings.config"], &expected); err != nil {
		t.Fatal(err)
	}
	expectedColors, ok := expected["filament_colour"].([]any)
	if !ok || len(expectedColors) != 5 {
		t.Fatalf("source must contain five explicit colors: %v", expected["filament_colour"])
	}
	expectedTypes, ok := expected["filament_type"].([]any)
	if !ok || len(expectedTypes) != 5 {
		t.Fatalf("source must contain five material types: %v", expected["filament_type"])
	}

	app := &App{}
	reply := app.Convert(ConvertRequest{FilePath: source, Targets: []string{"qidi"}})
	if !reply.OK || len(reply.Results) != 1 || len(reply.Failures) != 0 || !reply.Results[0].OK {
		t.Fatalf("real app QIDI conversion failed: %+v", reply)
	}
	result := reply.Results[0]
	if result.Report["target_filament_slots_used"] != 5 ||
		result.Report["source_filament_slots"] != 5 {
		t.Fatalf("incorrect five-material app contract: %+v", result.Report)
	}
	if _, special := result.Report["native_slicer_material_review_required"]; special {
		t.Fatalf("QIDI importer metadata compatibility must not add a special UI review field")
	}
	converted := appRead3MFParts(t, result.OutputPath)
	if !bytes.Contains(converted["3D/3dmodel.model"], []byte(`<metadata name="Application">QIDIStudio-01.09.03.51</metadata>`)) {
		t.Fatal("App.Convert output omitted automatic QIDI importer compatibility metadata")
	}
	var actual map[string]any
	if err := json.Unmarshal(converted["Metadata/project_settings.config"], &actual); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"filament_colour", "filament_type"} {
		if !reflect.DeepEqual(expected[field], actual[field]) {
			t.Fatalf("App.Convert changed the five source %s entries: src=%v got=%v", field, expected[field], actual[field])
		}
	}
	installedIDs, ok := actual["filament_settings_id"].([]any)
	if !ok || len(installedIDs) != 5 {
		t.Fatalf("QIDI installed material profile IDs missing: %v", actual["filament_settings_id"])
	}
	for _, id := range installedIDs {
		if text, ok := id.(string); !ok || text == "" {
			t.Fatalf("empty/unresolved installed QIDI material profile id: %v", installedIDs)
		}
	}
	if _, stale := converted["Metadata/plate_1.gcode"]; stale {
		t.Fatal("App.Convert output contains stale source G-code")
	}
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(original, after) {
		t.Fatal("App.Convert mutated the original 3MF")
	}
	t.Logf("APP_QIDI_FIVE_MATERIAL_PASS: %s, source colors=%v types=%v", result.OutputPath, expectedColors, expectedTypes)
}

func appRead3MFParts(t *testing.T, file string) map[string][]byte {
	t.Helper()
	reader, err := zip.OpenReader(file)
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	parts := make(map[string][]byte, len(reader.File))
	for _, member := range reader.File {
		if member.UncompressedSize64 > 64<<20 {
			t.Fatalf("oversized QA part %s", member.Name)
		}
		if _, repeated := parts[member.Name]; repeated {
			t.Fatalf("duplicate 3MF member %s", member.Name)
		}
		stream, err := member.Open()
		if err != nil {
			t.Fatal(err)
		}
		content, err := io.ReadAll(io.LimitReader(stream, 64<<20+1))
		closeErr := stream.Close()
		if err != nil || closeErr != nil {
			t.Fatalf("read %s: %v/%v", member.Name, err, closeErr)
		}
		parts[member.Name] = content
	}
	return parts
}
