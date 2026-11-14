package bridge

import (
	"archive/zip"
	"bytes"
	"encoding/xml"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestQIDIApplicationMetadataAutomaticAndIdempotent(t *testing.T) {
	for _, tc := range []struct{ name, input string }{
		{"missing", `<?xml version='1.0'?><model xmlns="urn:3mf"><resources/><build/></model>`},
		{"external", `<model><metadata name='Application'>BambuStudio-1.2.3</metadata><resources/></model>`},
		{"existing", `<model><metadata name="Application">QIDIStudio-01.09.03.51</metadata><resources/></model>`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			output, err := setQIDIApplicationMetadata([]byte(tc.input))
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Contains(output, []byte("<metadata name=\"Application\">"+qidiStudioApplication+"</metadata>")) {
				t.Fatalf("QIDI native importer metadata absent: %s", output)
			}
			if bytes.Count(output, []byte("name=\"Application\"")) != 1 {
				t.Fatalf("QIDI output has duplicate Application metadata: %s", output)
			}
			if strings.Contains(string(output), "BambuStudio-1.2.3") {
				t.Fatal("old generator application not replaced")
			}
			repeated, err := setQIDIApplicationMetadata(output)
			if err != nil || !bytes.Equal(output, repeated) {
				t.Fatalf("repeated conversion must not add or mutate an Application field: %v", err)
			}
		})
	}
	for _, invalid := range []string{
		"<model><metadata name='Application'>one</metadata><metadata name='Application'>two</metadata></model>",
		"<model><resources>",
		"<!DOCTYPE model [<!ENTITY x 'unsafe'>]><model/>",
		"<not-model/>",
		"<model/>",
	} {
		if _, err := setQIDIApplicationMetadata([]byte(invalid)); err == nil {
			t.Fatalf("invalid/duplicate 3MF origin XML did not fail: %s", invalid)
		}
	}
}

func TestQIDIAutomaticImporterMetadataIsTargetScoped(t *testing.T) {
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	qidi := filepath.Join(dir, "qidi.3mf")
	ordinary := filepath.Join(dir, "ordinary.3mf")
	fixture(t, source, 2)
	before, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := stripSliceCacheForQIDI(source, qidi, nil, true); err != nil {
		t.Fatal(err)
	}
	if _, err := stripSliceCache(source, ordinary, nil); err != nil {
		t.Fatal(err)
	}
	if got := project3MFApplication(t, qidi); got != qidiStudioApplication {
		t.Fatalf("QIDI output did not automatically receive native importer metadata: %q", got)
	}
	if got := project3MFApplication(t, ordinary); got == qidiStudioApplication {
		t.Fatal("ordinary non-QIDI converted project was assigned QIDI origin")
	}
	qidiParts := inspectZipEntries(t, qidi)
	ordinaryParts := inspectZipEntries(t, ordinary)
	if len(qidiParts) != len(ordinaryParts) {
		t.Fatalf("QIDI compatibility changed member count: %d vs %d", len(qidiParts), len(ordinaryParts))
	}
	for name, original := range ordinaryParts {
		converted, ok := qidiParts[name]
		if !ok {
			t.Fatalf("QIDI conversion lost ZIP member %q", name)
		}
		if name == "3D/3dmodel.model" {
			element := []byte("\n  <metadata name=\"Application\">QIDIStudio-01.09.03.51</metadata>")
			if bytes.Count(converted, element) != 1 ||
				!bytes.Equal(bytes.Replace(converted, element, nil, 1), original) {
				t.Fatalf("QIDI model changed beyond the single verified Application metadata element")
			}
		} else if !bytes.Equal(original, converted) {
			t.Fatalf("target-scoped importer compatibility altered %s", name)
		}
	}
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("QIDI compatibility changed the original source")
	}
}

func inspectZipEntries(t *testing.T, filename string) map[string][]byte {
	t.Helper()
	r, err := zip.OpenReader(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	parts := make(map[string][]byte, len(r.File))
	for _, entry := range r.File {
		if entry.UncompressedSize64 > 256<<20 {
			t.Fatalf("test archive part too large: %s", entry.Name)
		}
		if _, exists := parts[entry.Name]; exists {
			t.Fatalf("duplicate 3MF part: %s", entry.Name)
		}
		stream, err := entry.Open()
		if err != nil {
			t.Fatal(err)
		}
		raw, err := io.ReadAll(io.LimitReader(stream, 256<<20+1))
		closeErr := stream.Close()
		if err != nil || closeErr != nil {
			t.Fatalf("read ZIP part %s: %v, %v", entry.Name, err, closeErr)
		}
		parts[entry.Name] = raw
	}
	return parts
}

func TestQIDISanitizerRejectsAbsentOrRepeatedPrimaryModel(t *testing.T) {
	for _, tc := range []struct {
		name   string
		models int
	}{
		{"missing", 0},
		{"duplicate", 2},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			input := filepath.Join(dir, "source.3mf")
			f, err := os.OpenFile(input, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
			if err != nil {
				t.Fatal(err)
			}
			zw := zip.NewWriter(f)
			for i := 0; i < tc.models; i++ {
				dst, err := zw.Create("3D/3dmodel.model")
				if err != nil {
					t.Fatal(err)
				}
				if _, err = dst.Write([]byte("<model><resources/><build/></model>")); err != nil {
					t.Fatal(err)
				}
			}
			dst, err := zw.Create("Metadata/project_settings.config")
			if err != nil {
				t.Fatal(err)
			}
			if _, err = dst.Write([]byte("{}")); err != nil {
				t.Fatal(err)
			}
			if err = zw.Close(); err != nil {
				t.Fatal(err)
			}
			if err = f.Close(); err != nil {
				t.Fatal(err)
			}
			if _, err = stripSliceCacheForQIDI(input, filepath.Join(dir, "result.3mf"), nil, true); err == nil {
				t.Fatalf("QIDI conversion accepted 3MF with %d primary model parts", tc.models)
			}
		})
	}
}

func project3MFApplication(t *testing.T, filename string) string {
	t.Helper()
	zr, err := zip.OpenReader(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer zr.Close()
	var model *zip.File
	for _, part := range zr.File {
		if part.Name == "3D/3dmodel.model" {
			if model != nil {
				t.Fatal("duplicate primary model part in 3MF")
			}
			model = part
		}
	}
	if model == nil {
		t.Fatal("missing 3MF model XML")
	}
	if model.UncompressedSize64 > 8<<20 {
		t.Fatal("oversized test model XML")
	}
	stream, err := model.Open()
	if err != nil {
		t.Fatal(err)
	}
	raw, err := io.ReadAll(io.LimitReader(stream, 8<<20+1))
	closeErr := stream.Close()
	if err != nil || closeErr != nil {
		t.Fatalf("read primary model %v, %v", err, closeErr)
	}
	var m struct {
		XMLName  xml.Name `xml:"model"`
		Metadata []struct {
			Name  string `xml:"name,attr"`
			Value string `xml:",chardata"`
		} `xml:"metadata"`
	}
	if err := xml.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	if m.XMLName.Local != "model" {
		t.Fatal("expected model root")
	}
	for _, meta := range m.Metadata {
		if meta.Name == "Application" {
			return strings.TrimSpace(meta.Value)
		}
	}
	return ""
}

func TestQIDIApplicationMetadataRetainsQualifiedModelNamespace(t *testing.T) {
	for name, original := range map[string]string{
		"missing":  `<?xml version="1.0"?><p:model xmlns:p="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><p:resources/><p:build/></p:model>`,
		"existing": `<p:model xmlns:p="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><p:metadata name="Application">OldTool-0.2</p:metadata><p:build/></p:model>`,
	} {
		t.Run(name, func(t *testing.T) {
			updated, err := setQIDIApplicationMetadata([]byte(original))
			if err != nil {
				t.Fatal(err)
			}
			expected := []byte(`<p:metadata name="Application">QIDIStudio-01.09.03.51</p:metadata>`)
			if bytes.Count(updated, expected) != 1 {
				t.Fatalf("metadata lost its 3MF namespace: %s", updated)
			}
			var parsed struct {
				XMLName xml.Name `xml:"model"`
			}
			if err := xml.Unmarshal(updated, &parsed); err != nil || parsed.XMLName.Local != "model" {
				t.Fatalf("qualified QIDI model XML became invalid: %v (root %q)", err, parsed.XMLName.Local)
			}
			repeated, err := setQIDIApplicationMetadata(updated)
			if err != nil || !bytes.Equal(updated, repeated) {
				t.Fatalf("qualified Application update was not idempotent: %v", err)
			}
		})
	}
}
