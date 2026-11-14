package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

// TestRustArchivePreserveParity is an opt-in cross-language check for the
// process-setting preservation branch of Go stripSliceCache.
// Outputs are created in temporary directories; no installed app is changed.
func TestRustArchivePreserveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe binary")
	}
	sources, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "*.3mf"))
	if err != nil || len(sources) == 0 {
		t.Fatalf("missing fixtures: %v", err)
	}
	preserved := []string{"filament_flow_ratio", "layer_height", "filament_flow_ratio"}
	for _, source := range sources {
		t.Run(filepath.Base(source), func(t *testing.T) {
			tmp := t.TempDir()
			goOutput := filepath.Join(tmp, "go.3mf")
			rustOutput := filepath.Join(tmp, "rust.3mf")

			removed, goErr := stripSliceCache(source, goOutput, preserved)
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			args := []string{"--strip-cache"}
			for _, key := range preserved {
				args = append(args, "--preserve", key)
			}
			args = append(args, source, rustOutput)
			out, rustErr := exec.CommandContext(ctx, rust, args...).CombinedOutput()
			if ctx.Err() != nil {
				t.Fatal(ctx.Err())
			}
			var response struct {
				OK      bool   `json:"ok"`
				Error   string `json:"error"`
				Removed int    `json:"removed"`
			}
			if err := json.Unmarshal(out, &response); err != nil {
				t.Fatalf("Rust response invalid: %v (%q)", err, out)
			}
			if goErr != nil {
				if rustErr == nil || response.OK {
					t.Fatalf("Go rejected %s (%v), Rust accepted (%s)", source, goErr, out)
				}
				return
			}
			if rustErr != nil || !response.OK {
				t.Fatalf("Go accepted %s, Rust rejected (%v): %s", source, rustErr, out)
			}
			if response.Removed != removed {
				t.Fatalf("removed count differs: Go=%d Rust=%d", removed, response.Removed)
			}

			goParts := inspectZipEntries(t, goOutput)
			rustParts := inspectZipEntries(t, rustOutput)
			if len(goParts) != len(rustParts) {
				t.Fatalf("ZIP part count differs: Go=%d Rust=%d", len(goParts), len(rustParts))
			}
			for name, goContent := range goParts {
				rustContent, found := rustParts[name]
				if !found {
					t.Fatalf("Rust missing part %q", name)
				}
				switch {
				case name == "Metadata/project_settings.config":
					var a, b any
					if err := json.Unmarshal(goContent, &a); err != nil {
						t.Fatalf("Go JSON invalid: %v", err)
					}
					if err := json.Unmarshal(rustContent, &b); err != nil {
						t.Fatalf("Rust JSON invalid: %v", err)
					}
					if !reflect.DeepEqual(a, b) {
						t.Fatalf("setting metadata differs:\nGo: %s\nRust: %s", goContent, rustContent)
					}
				case name == "[Content_Types].xml" || strings.HasSuffix(name, ".rels"):
					if !reflect.DeepEqual(
						semanticXMLTokens(t, goContent),
						semanticXMLTokens(t, rustContent),
					) {
						t.Fatalf("OPC XML differs in %q", name)
					}
				default:
					if !bytes.Equal(goContent, rustContent) {
						t.Fatalf("payload differs in %q", name)
					}
				}
			}
			rustFacts, err := inspectPackage(rustOutput)
			if err != nil {
				t.Fatalf("Go inspector rejected Rust ZIP: %v", err)
			}
			if rustFacts.sliceCaches != 0 {
				t.Fatalf("stale cache entries remain: %d", rustFacts.sliceCaches)
			}
		})
	}
}
