package bridge

import (
	"archive/zip"
	"encoding/json"
	"encoding/xml"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// This opt-in historical QA test checks the original pre-fix converter
// artifacts. It does not describe the currently converted QIDI outputs.
// It reproduces vendor import classification without launching the slicer or
// touching the printer. QIDIStudio source qds_3mf.cpp checks whether
// the 3MF Application metadata starts with "QIDIStudio-" to set
// m_is_qdt_3mf. The installed calibration fixture is only a genuine
// QIDIStudio PRODUCER CONTROL: it targets an unrelated QIDI model,
// and is never used as a printer profile or copied into an output.
func TestQIDIPreFixExternalProjectOriginAndFiveFilamentPalette(t *testing.T) {
	vendor := os.Getenv("SLICEBRIDGE_QIDI_VENDOR_CALIBRATION_3MF")
	outputs := os.Getenv("SLICEBRIDGE_QIDI_FIVE_SLOT_OUTPUT_DIR")
	if vendor == "" || outputs == "" {
		t.Skip("opt-in: provide read-only QIDIStudio calibration 3MF and mixed-five scratch output folder")
	}
	official := project3MFApplication(t, vendor)
	if !strings.HasPrefix(official, "QIDIStudio-") {
		t.Fatalf("genuine installed QIDIStudio calibration 3MF lacks native Application provenance: %q", official)
	}
	for _, name := range []string{
		"go-qidi-x-plus-5-center-4PLA-1PETG.3mf",
		"rust-qidi-x-plus-5-center-4PLA-1PETG.3mf",
	} {
		t.Run(name, func(t *testing.T) {
			path := filepath.Join(outputs, name)
			current := project3MFApplication(t, path)
			if strings.HasPrefix(current, "QIDIStudio-") {
				t.Fatalf("historical pre-fix conversion artifact no longer matches its source receipt: %q", current)
			}
			// Vendor logic will classify each as From_Other, regardless of
			// the correct five material settings in this non-native archive.
			r, err := zip.OpenReader(path)
			if err != nil {
				t.Fatal(err)
			}
			defer r.Close()
			entry := zipPartByName(r.File, "Metadata/project_settings.config")
			if entry == nil {
				t.Fatal("missing converted printer/material metadata")
			}
			rc, err := entry.Open()
			if err != nil {
				t.Fatal(err)
			}
			raw, err := io.ReadAll(io.LimitReader(rc, 4<<20))
			_ = rc.Close()
			if err != nil {
				t.Fatal(err)
			}
			var data map[string]any
			if err := json.Unmarshal(raw, &data); err != nil {
				t.Fatal(err)
			}
			if data["printer_model"] != "X-Plus 5" {
				t.Fatalf("output already lost target printer settings: %v", data["printer_model"])
			}
			wantMaterials := []string{"PLA", "PLA", "PLA", "PLA", "PETG"}
			wantColors := []string{"#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FFFFFF"}
			for key, want := range map[string][]string{
				"filament_type":   wantMaterials,
				"filament_colour": wantColors,
			} {
				values, ok := data[key].([]any)
				if !ok || len(values) != len(want) {
					t.Fatalf("%s palette unexpectedly absent: %v", key, data[key])
				}
				for i, expected := range want {
					if values[i] != expected {
						t.Fatalf("%s slot %d changed: %v != %q", key, i+1, values[i], expected)
					}
				}
			}
			t.Logf("HISTORICAL_PRE_FIX_FOREIGN_3MF: five source materials were preserved but the former output lacked the importer metadata, Application=%q", current)
		})
	}
}

// Validates actual 3MF model XML rather than blindly matching marker bytes
// in an unrelated zip member, and never fabricates an Application property.
func project3MFApplication(t *testing.T, path string) string {
	t.Helper()
	r, err := zip.OpenReader(path)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	part := zipPartByName(r.File, "3D/3dmodel.model")
	if part == nil {
		t.Fatalf("missing 3MF model XML: %s", path)
	}
	in, err := part.Open()
	if err != nil {
		t.Fatal(err)
	}
	raw, err := io.ReadAll(io.LimitReader(in, 8<<20))
	_ = in.Close()
	if err != nil {
		t.Fatal(err)
	}
	var m struct {
		XMLName  xml.Name `xml:"model"`
		Metadata []struct {
			Name  string `xml:"name,attr"`
			Value string `xml:",chardata"`
		} `xml:"metadata"`
	}
	if err := xml.Unmarshal(raw, &m); err != nil {
		t.Fatalf("invalid 3MF model XML %s: %v", path, err)
	}
	if m.XMLName.Local != "model" {
		t.Fatal("not a 3MF model XML")
	}
	for _, meta := range m.Metadata {
		if meta.Name == "Application" {
			return strings.TrimSpace(meta.Value)
		}
	}
	return ""
}
func zipPartByName(files []*zip.File, name string) *zip.File {
	for _, file := range files {
		if file.Name == name {
			return file
		}
	}
	return nil
}
