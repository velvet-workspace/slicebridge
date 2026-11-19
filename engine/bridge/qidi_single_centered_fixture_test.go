package bridge

import (
	"archive/zip"
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// createPinnedCenteredOneSource prepares a *one-filament* synthetic 3MF for
// owner GUI inspection of QIDIStudio without BOX. Only the build-item
// position is changed; mesh triangles, paint tokens, spool type and color
// are byte-identical to the pinned source. It never edits an installed file.
func createPinnedCenteredOneSource(t *testing.T, destination string) {
	t.Helper()
	pinned := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
	source, err := zip.OpenReader(pinned)
	if err != nil {
		t.Fatal(err)
	}
	defer source.Close()

	output, err := os.OpenFile(destination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		t.Fatal(err)
	}
	writer := zip.NewWriter(output)
	modelParts := 0
	for _, part := range source.File {
		body, err := readSyntheticZipEntry(part)
		if err != nil {
			t.Fatal(err)
		}
		if part.Name == "3D/3dmodel.model" {
			oldBuild := []byte("<build><item objectid='1'/></build>")
			if bytes.Count(body, oldBuild) != 1 {
				t.Fatal("pinned one-filament mesh/build item changed unexpectedly")
			}
			body = bytes.Replace(body, oldBuild,
				[]byte("<build><item objectid='1' transform='1 0 0 0 1 0 0 0 1 120 120 0'/></build>"), 1)
			modelParts++
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
	if modelParts != 1 {
		t.Fatalf("pinned 3MF model count changed: %d", modelParts)
	}
	inspection, err := converter.Inspect(destination)
	if err != nil || len(inspection.Filaments) != 1 ||
		inspection.Filaments[0].Type != "PLA" || inspection.Filaments[0].Color != "#FF0000" {
		t.Fatalf("one-spool synthetic source changed or failed inspection: %+v / %v", inspection, err)
	}
	t.Log("centered pinned source: one filament and one source virtual palette entry; mesh unchanged")
}
