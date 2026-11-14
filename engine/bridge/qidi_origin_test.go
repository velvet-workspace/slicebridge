package bridge

import (
	"bytes"
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
			want, err := setQIDIApplicationMetadata(original)
			if err != nil || !bytes.Equal(converted, want) {
				t.Fatalf("QIDI model changed beyond one Application field: %v", err)
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
