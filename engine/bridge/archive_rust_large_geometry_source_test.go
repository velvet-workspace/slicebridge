package bridge

import (
	"archive/zip"
	"bufio"
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"
)

// createBoundedTwoPlateGeometrySource replaces the small pinned mesh with
// a deterministic grid of disjoint closed tetrahedra. This tests actual
// streamed 3D vertices/triangles rather than a compressible XML comment.
// Both synthetic plates declare the same material but carry separate usage,
// plus independent stale cache entries which must disappear in converted
// outputs. Not a user project, not a slicer-ready production benchmark.
func createBoundedTwoPlateGeometrySource(t *testing.T, target string, solids int) {
	t.Helper()
	if solids < 15000 || solids > 30000 {
		t.Fatal("synthetic geometry test must remain within 15k..30k tetrahedra")
	}
	pinned := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
	original, err := zip.OpenReader(pinned)
	if err != nil {
		t.Fatal(err)
	}
	defer original.Close()
	file, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal(err)
	}
	output := zip.NewWriter(file)
	foundModel, foundSliceInfo := false, false
	for _, entry := range original.File {
		// Create a fresh header: the copied input CRC/sizes are invalid for rewritten geometry.
		dst, err := output.Create(entry.Name)
		if err != nil {
			t.Fatal(err)
		}
		switch entry.Name {
		case "3D/3dmodel.model":
			if foundModel {
				t.Fatal("duplicate model")
			}
			foundModel = true
			buffered := bufio.NewWriterSize(dst, 1<<16)
			if _, err := io.WriteString(buffered,
				"<?xml version='1.0' encoding='UTF-8'?>"+
					"<model xmlns='http://schemas.microsoft.com/3dmanufacturing/core/2015/02' unit='millimeter'>"+
					"<resources><object id='1' type='model'><mesh><vertices>"); err != nil {
				t.Fatal(err)
			}
			for i := 0; i < solids; i++ {
				x, y := i%200, i/200
				if _, err := fmt.Fprintf(buffered,
					"<vertex x='%d' y='%d' z='0'/>"+
						"<vertex x='%d.45' y='%d' z='0'/>"+
						"<vertex x='%d' y='%d.45' z='0'/>"+
						"<vertex x='%d' y='%d' z='0.45'/>",
					x, y, x, y, x, y, x, y); err != nil {
					t.Fatal(err)
				}
			}
			if _, err := io.WriteString(buffered, "</vertices><triangles>"); err != nil {
				t.Fatal(err)
			}
			for i := 0; i < solids; i++ {
				a := 4 * i
				// Four triangular faces per tetrahedron form one closed
				// unit. Distinct vertex coordinates exercise realistic
				// model XML streaming and per-triangle paint rewrites.
				if _, err := fmt.Fprintf(buffered,
					"<triangle v1='%d' v2='%d' v3='%d' paint_color='4'/>"+
						"<triangle v1='%d' v2='%d' v3='%d' paint_color='4'/>"+
						"<triangle v1='%d' v2='%d' v3='%d' paint_color='4'/>"+
						"<triangle v1='%d' v2='%d' v3='%d' paint_color='4'/>",
					a, a+1, a+2, a, a+1, a+3, a, a+2, a+3, a+1, a+2, a+3); err != nil {
					t.Fatal(err)
				}
			}
			if _, err := io.WriteString(buffered,
				"</triangles></mesh></object></resources><build><item objectid='1'/></build></model>"); err != nil {
				t.Fatal(err)
			}
			if err := buffered.Flush(); err != nil {
				t.Fatal(err)
			}
		case "Metadata/slice_info.config":
			if foundSliceInfo {
				t.Fatal("duplicate slice-info")
			}
			foundSliceInfo = true
			body, err := readSyntheticZipEntry(entry)
			if err != nil {
				t.Fatal(err)
			}
			second := []byte("<plate><metadata key='printer_model_id' value='C13'/>" +
				"<filament id='1' type='PLA' color='#FF0000' used_m='2' used_g='5'/></plate></config>")
			if bytes.Count(body, []byte("</config>")) != 1 {
				t.Fatal("pinned slice info root changed")
			}
			body = bytes.Replace(body, []byte("</config>"), second, 1)
			if bytes.Count(body, []byte("<plate>")) != 2 {
				t.Fatal("did not create exactly 2 independent slice-info plates")
			}
			if _, err := dst.Write(body); err != nil {
				t.Fatal(err)
			}
		default:
			content, err := readSyntheticZipEntry(entry)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := dst.Write(content); err != nil {
				t.Fatal(err)
			}
		}
	}
	if !foundModel || !foundSliceInfo {
		t.Fatal("source pinned package omitted expected model or plate metadata")
	}
	// A second cached plate must be sanitized exactly like the first.
	for _, entry := range []struct {
		name    string
		content string
	}{
		{"Metadata/plate_2.gcode", "; stale test-only cache, never executed"},
		{"Metadata/plate_2.gcode.md5", "stale"},
		{"Metadata/plate_2.json", "{}"},
	} {
		writer, err := output.Create(entry.name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := io.WriteString(writer, entry.content); err != nil {
			t.Fatal(err)
		}
	}
	if err := output.Close(); err != nil {
		t.Fatal(err)
	}
	if err := file.Sync(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}

func readSyntheticZipEntry(entry *zip.File) ([]byte, error) {
	const limit = 8 << 20
	if entry.UncompressedSize64 > limit {
		return nil, fmt.Errorf("pinned source entry %q is unexpectedly large", entry.Name)
	}
	reader, err := entry.Open()
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	body, err := io.ReadAll(io.LimitReader(reader, limit+1))
	if err != nil {
		return nil, err
	}
	if len(body) > limit {
		return nil, fmt.Errorf("pinned source entry %q exceeds bounded reader", entry.Name)
	}
	return body, nil
}
