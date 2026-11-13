package bridge

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// createPinnedNativeReadableMixedFiveSource clones the already-native-readable
// pinned Bambu 3MF tetrahedron into a new exclusive scratch file and changes
// ONLY the fifth declared material from PLA to PETG. The four-vertex,
// four-facet mesh and its original paint/default references stay byte-identical.
// This deliberately avoids the geometry of a separate generated fixture that
// the native Snapmaker Orca CLI rejected before any 3MF material comparison.
func createPinnedNativeReadableMixedFiveSource(t *testing.T, destination string) {
	t.Helper()
	pinned := filepath.Join("..", "..", "tests", "fixtures", "bambu-five-colors.3mf")
	reader, err := zip.OpenReader(pinned)
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal(err)
	}
	writer := zip.NewWriter(output)
	foundSettings := false
	foundPalette := false
	for _, part := range reader.File {
		src, err := part.Open()
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(src)
		_ = src.Close()
		if err != nil {
			t.Fatal(err)
		}
		switch part.Name {
		case "Metadata/project_settings.config":
			if foundSettings {
				t.Fatal("duplicate source project settings")
			}
			var settings map[string]any
			if err := json.Unmarshal(body, &settings); err != nil {
				t.Fatal(err)
			}
			materials, ok := settings["filament_type"].([]any)
			if !ok || len(materials) != 5 || materials[4] != "PLA" {
				t.Fatalf("pinned native-readable palette changed: %v", materials)
			}
			materials[4] = "PETG"
			body, err = json.Marshal(settings)
			if err != nil {
				t.Fatal(err)
			}
			foundSettings = true
		case "Metadata/slice_info.config":
			if foundPalette {
				t.Fatal("duplicate source slice palette")
			}
			original := []byte("id='5' type='PLA'")
			if bytes.Count(body, original) != 1 {
				t.Fatal("pinned native-readable fifth filament changed")
			}
			body = bytes.Replace(body, original, []byte("id='5' type='PETG'"), 1)
			foundPalette = true
		}
		header := part.FileHeader
		dst, err := writer.CreateHeader(&header)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := dst.Write(body); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := output.Close(); err != nil {
		t.Fatal(err)
	}
	if !foundSettings || !foundPalette {
		t.Fatalf("pinned native-readable 3MF missing required material metadata: settings=%t palette=%t", foundSettings, foundPalette)
	}
	inspection, err := converter.Inspect(destination)
	if err != nil {
		t.Fatalf("repainted pinned 3MF inspection failed: %v", err)
	}
	if len(inspection.Filaments) != 5 || inspection.Filaments[4].Type != "PETG" {
		t.Fatalf("source fifth material not independently inspected as PETG: %v", inspection.Filaments)
	}
	t.Log(fmt.Sprintf("reused native-readable pinned 3MF tetrahedron; fifth material changed to %s; original mesh unchanged", inspection.Filaments[4].Type))
}
