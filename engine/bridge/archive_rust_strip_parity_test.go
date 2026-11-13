package bridge

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"testing"
	"time"
)

// TestRustArchiveStripParity checks extracted ZIP parts against the production
// Go implementation. It is opt-in, never used by the native app, and never
// operates on a user file (all outputs are in t.TempDir()).
func TestRustArchiveStripParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe binary")
	}
	fixtures, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "*.3mf"))
	if err != nil || len(fixtures) == 0 {
		t.Fatalf("3MF fixtures unavailable: %v", err)
	}
	for _, source := range fixtures {
		t.Run(filepath.Base(source), func(t *testing.T) {
			base := t.TempDir()
			goPath := filepath.Join(base, "go.3mf")
			rustPath := filepath.Join(base, "rust.3mf")
			goRemoved, goErr := stripSliceCache(source, goPath, nil)

			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			out, rustErr := exec.CommandContext(ctx, rust, "--strip-cache", source, rustPath).CombinedOutput()
			if ctx.Err() != nil {
				t.Fatalf("Rust cache strip timed out: %v", ctx.Err())
			}
			var reply struct {
				OK      bool   `json:"ok"`
				Error   string `json:"error"`
				Removed int    `json:"removed"`
			}
			if err := json.Unmarshal(out, &reply); err != nil {
				t.Fatalf("non-JSON Rust response: %v (%q)", err, out)
			}
			if goErr != nil {
				if rustErr == nil || reply.OK {
					t.Fatalf("Go rejected source (%v), Rust accepted (%s)", goErr, out)
				}
				return
			}
			if rustErr != nil || !reply.OK {
				t.Fatalf("Go accepted source, Rust rejected: %v (%s)", rustErr, out)
			}
			if reply.Removed != goRemoved {
				t.Fatalf("Go removed %d cache parts; Rust removed %d", goRemoved, reply.Removed)
			}
			goParts := inspectZipEntries(t, goPath)
			rustParts := inspectZipEntries(t, rustPath)
			if len(goParts) != len(rustParts) {
				t.Fatalf("ZIP entry count differs: Go=%d, Rust=%d", len(goParts), len(rustParts))
			}
			for name, goContent := range goParts {
				rustContent, exists := rustParts[name]
				if !exists {
					t.Fatalf("Rust missing ZIP part %q", name)
				}
				if name == "[Content_Types].xml" || strings.HasSuffix(name, ".rels") {
					// Writers may choose different whitespace and namespace
					// declarations; compare parsed element/attribute meaning.
					left := semanticXMLTokens(t, goContent)
					right := semanticXMLTokens(t, rustContent)
					if !reflect.DeepEqual(left, right) {
						t.Fatalf("OPC XML differs at %s\nGo: %v\nRust: %v", name, left, right)
					}
				} else if !bytes.Equal(goContent, rustContent) {
					t.Fatalf("ZIP part payload differs: %s", name)
				}
			}
			// Re-validate the Rust result using the production Go inspector.
			facts, err := inspectPackage(rustPath)
			if err != nil {
				t.Fatalf("production Go inspector rejects Rust output: %v", err)
			}
			if facts.sliceCaches != 0 {
				t.Fatalf("Rust left %d sliced cache parts", facts.sliceCaches)
			}
		})
	}
}

func inspectZipEntries(t *testing.T, filename string) map[string][]byte {
	t.Helper()
	zr, err := zip.OpenReader(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer zr.Close()
	out := make(map[string][]byte, len(zr.File))
	for _, file := range zr.File {
		rc, err := file.Open()
		if err != nil {
			t.Fatal(err)
		}
		content, err := io.ReadAll(io.LimitReader(rc, 256<<20))
		rc.Close()
		if err != nil {
			t.Fatal(err)
		}
		if _, exists := out[file.Name]; exists {
			t.Fatalf("duplicate ZIP entry %q", file.Name)
		}
		out[file.Name] = content
	}
	return out
}

func semanticXMLTokens(t *testing.T, data []byte) []string {
	t.Helper()
	decoder := xml.NewDecoder(bytes.NewReader(data))
	var tokens []string
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			return tokens
		}
		if err != nil {
			t.Fatalf("invalid OPC XML: %v (%q)", err, data)
		}
		switch node := token.(type) {
		case xml.StartElement:
			var attrs []string
			for _, attr := range node.Attr {
				// Namespace declaration is covered by namespace-resolved
				// element names and does not need a separate text token.
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
			if trimmed := strings.TrimSpace(string(node)); trimmed != "" {
				tokens = append(tokens, "TEXT "+trimmed)
			}
		}
	}
}
