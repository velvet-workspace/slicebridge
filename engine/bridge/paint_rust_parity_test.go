package bridge

import (
	"bufio"
	"bytes"
	"encoding/xml"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/paint"
)

// The bitstream corpus comes from actual Bambu/Orca painted triangles checked
// into the existing Go dependency. Compare every rewritten hex string with
// the production Go codec, not a second hand-written reference algorithm.
func TestRustPaintCodecCorpusParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	filename := filepath.Join("..", "..", "third_party", "bl2std", "pkg", "paint", "testdata", "paint_samples.txt")
	file, err := os.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 64*1024), 1<<20)
	var corpus []string
	for scanner.Scan() {
		if s := strings.TrimSpace(scanner.Text()); s != "" {
			corpus = append(corpus, s)
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	if len(corpus) < 250 {
		t.Fatalf("paint corpus too small: %d", len(corpus))
	}

	for _, tc := range []struct {
		name    string
		mapping map[int]int
	}{
		{"identity", map[int]int{}},
		{"permutations", map[int]int{1: 2, 2: 1, 3: 1, 4: 3, 5: 4, 6: 5, 16: 3}},
		{"merge_to_first", map[int]int{1: 1, 2: 1, 3: 1, 4: 1, 5: 1, 16: 1}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			mapping := map[string]int{}
			for k, v := range tc.mapping {
				mapping[strconv.Itoa(k)] = v
			}
			response, err := runRustMaterialCommand(t, rust, "--remap-paint", map[string]any{
				"mapping": mapping,
				"samples": corpus,
			})
			if err != nil || response["ok"] != true {
				t.Fatalf("Rust paint codec failed: err=%v reply=%v", err, response)
			}
			raw, ok := response["samples"].([]any)
			if !ok || len(raw) != len(corpus) {
				t.Fatalf("Rust returned %d paint strings; expected %d", len(raw), len(corpus))
			}
			for i, sample := range corpus {
				want, err := paint.Remap(sample, func(filament int) int {
					if target, exists := tc.mapping[filament]; exists {
						return target
					}
					return filament
				})
				if err != nil {
					t.Fatalf("Go failed to decode corpus item %d: %v", i, err)
				}
				if got, ok := raw[i].(string); !ok || got != want {
					t.Fatalf("paint %d differs: original=%q Go=%q Rust=%q", i, sample, want, got)
				}
			}
		})
	}
}

// Verify an XML model including both known paint attributes. The model
// rewriting stage must not touch vertex indices or unpainted metadata.
func TestRustModelTrianglePaintParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	model := `<?xml version="1.0" encoding="UTF-8"?>
<model xmlns="urn:core" xmlns:p="urn:production"><resources><object id="1"><mesh>
<vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
<triangles><triangle v1="0" v2="1" v3="2" paint_color="4882" mmu_segmentation="8"/>
<triangle v1="2" v2="0" v3="1" paint_color="DC" custom="A&amp;B" />
<p:triangle v1="1" v2="2" v3="0" paint_color="0C" /></triangles>
</mesh></object></resources><metadata paint_color="4"/></model>`
	mapping := map[int]int{1: 2, 2: 1, 3: 1, 16: 3}
	keys := map[string]int{}
	for k, v := range mapping {
		keys[strconv.Itoa(k)] = v
	}
	reply, err := runRustMaterialCommand(t, rust, "--remap-model", map[string]any{
		"mapping":   keys,
		"model_xml": model,
	})
	if err != nil || reply["ok"] != true {
		t.Fatalf("Rust model rewriting failed: %v %v", err, reply)
	}
	output, ok := reply["model_xml"].(string)
	if !ok {
		t.Fatal("no rewritten model XML returned")
	}
	sourceAttrs := triangleAttributes(t, []byte(model))
	targetAttrs := triangleAttributes(t, []byte(output))
	if len(sourceAttrs) != len(targetAttrs) || len(sourceAttrs) != 3 {
		t.Fatalf("model triangle count changed: source=%d target=%d", len(sourceAttrs), len(targetAttrs))
	}
	for i, source := range sourceAttrs {
		got := targetAttrs[i]
		for name, value := range source {
			want := value
			if (name == "paint_color" || name == "mmu_segmentation") && value != "" {
				var err error
				want, err = paint.Remap(value, func(filament int) int {
					if mapped, ok := mapping[filament]; ok {
						return mapped
					}
					return filament
				})
				if err != nil {
					t.Fatal(err)
				}
			}
			if got[name] != want {
				t.Fatalf("triangle %d attr %q differs: Go=%q Rust=%q", i, name, want, got[name])
			}
		}
	}
	if !bytes.Contains([]byte(output), []byte(`custom="A&amp;B"`)) {
		t.Fatal("non-paint XML attribute escaping changed")
	}
	if !strings.Contains(output, `<metadata paint_color="4"/>`) {
		t.Fatal("non-triangle paint attribute changed")
	}
}

func triangleAttributes(t *testing.T, raw []byte) []map[string]string {
	t.Helper()
	decoder := xml.NewDecoder(bytes.NewReader(raw))
	var result []map[string]string
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatalf("model XML invalid: %v", err)
		}
		if el, ok := token.(xml.StartElement); ok && el.Name.Local == "triangle" {
			attrs := map[string]string{}
			for _, attr := range el.Attr {
				attrs[attr.Name.Local] = attr.Value
			}
			result = append(result, attrs)
		}
	}
	return result
}
