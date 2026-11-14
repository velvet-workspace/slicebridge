package bridge

import (
	"archive/zip"
	"bytes"
	"io"
	"os"
	"path/filepath"
	"testing"
)

// TestQIDIApplicationMarkerABExactDelta checks the isolated owner-verified
// QIDIStudio GUI A/B probe: only model-level Application metadata changed.
// This is a simulated vendor-author marker for diagnostic use, NOT a valid
// native project authoring API, not a shipping fix and not print-ready.
// Native GUI palette visibility was confirmed separately by the owner.
func TestQIDIApplicationMarkerABExactDelta(t *testing.T) {
	originals := os.Getenv("SLICEBRIDGE_QIDI_FIVE_SLOT_OUTPUT_DIR")
	variants := os.Getenv("SLICEBRIDGE_QIDI_MARKER_AB_DIR")
	if originals == "" || variants == "" {
		t.Skip("opt-in: set original and marker-only scratch 3MF directories")
	}
	const producerTag = "QIDIStudio-01.09.03.51"
	injected := []byte("\n  <metadata name=\"Application\">" + producerTag + "</metadata>")
	for _, engine := range []string{"go", "rust"} {
		t.Run(engine, func(t *testing.T) {
			original := filepath.Join(originals, engine+"-qidi-x-plus-5-center-4PLA-1PETG.3mf")
			variant := filepath.Join(variants, "EXPERIMENTAL-MARKER-ONLY-DO-NOT-PRINT-"+engine+"-qidi-4PLA-1PETG.3mf")
			if got := project3MFApplication(t, original); got != "" {
				t.Fatalf("baseline already has a native producer tag: %q", got)
			}
			if got := project3MFApplication(t, variant); got != producerTag {
				t.Fatalf("unexpected A/B producer tag: %q", got)
			}
			before, err := zip.OpenReader(original)
			if err != nil {
				t.Fatal(err)
			}
			defer before.Close()
			after, err := zip.OpenReader(variant)
			if err != nil {
				t.Fatal(err)
			}
			defer after.Close()
			if len(before.File) != len(after.File) {
				t.Fatalf("archive member count changed: %d -> %d", len(before.File), len(after.File))
			}
			for i, part := range before.File {
				if part.Name != after.File[i].Name {
					t.Fatalf("archive member order or name changed: %s != %s", part.Name, after.File[i].Name)
				}
				originalPart := readZipPartForAB(t, part)
				variantPart := readZipPartForAB(t, after.File[i])
				if part.Name != "3D/3dmodel.model" {
					if !bytes.Equal(originalPart, variantPart) {
						t.Fatalf("non-model ZIP member was altered: %s", part.Name)
					}
					continue
				}
				if bytes.Count(variantPart, injected) != 1 {
					t.Fatal("exact experimental marker must be inserted once")
				}
				if !bytes.Equal(originalPart, bytes.Replace(variantPart, injected, nil, 1)) {
					t.Fatal("model changed beyond one injected Application metadata tag")
				}
			}
			t.Logf("PASS %s: one diagnostic Application insertion, no changes to material profiles, mesh, bed position, or other 3MF members; GUI parity observed independently", engine)
		})
	}
}

func readZipPartForAB(t *testing.T, part *zip.File) []byte {
	t.Helper()
	handle, err := part.Open()
	if err != nil {
		t.Fatal(err)
	}
	defer handle.Close()
	// The owner test fixture contains only tiny 3MF parts; refuse to
	// silently compare huge untrusted archives as part of this opt-in QA.
	if part.UncompressedSize64 > 8<<20 {
		t.Fatalf("oversized probe member: %s", part.Name)
	}
	buf := make([]byte, part.UncompressedSize64)
	if _, err := io.ReadFull(handle, buf); err != nil {
		t.Fatal(err)
	}
	return buf
}
