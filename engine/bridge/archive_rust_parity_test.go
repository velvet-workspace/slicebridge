package bridge

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

// TestRustArchiveProbeParity is opt-in: it compares the experimental Rust
// read-only archive probe with the actual production Go inspectPackage.
// It does not change conversion behavior, package releases, or send prints.
func TestRustArchiveProbeParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe binary")
	}
	sources, err := filepath.Glob(filepath.Join("..", "..", "tests", "fixtures", "*.3mf"))
	if err != nil || len(sources) == 0 {
		t.Fatalf("missing 3MF fixture corpus: %v", err)
	}
	for _, source := range sources {
		t.Run(filepath.Base(source), func(t *testing.T) {
			expected, goErr := inspectPackage(source)
			ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()
			out, processErr := exec.CommandContext(ctx, rust, source).CombinedOutput()
			if ctx.Err() != nil {
				t.Fatalf("Rust probe timed out: %v", ctx.Err())
			}
			var response struct {
				OK    bool   `json:"ok"`
				Error string `json:"error"`
				Facts struct {
					Vertices     int    `json:"vertices"`
					Triangles    int    `json:"triangles"`
					SliceCaches  int    `json:"slice_caches"`
					PrinterModel string `json:"printer_model"`
				} `json:"facts"`
			}
			if err := json.Unmarshal(out, &response); err != nil {
				t.Fatalf("Rust response not JSON: %v (%q)", err, out)
			}
			if goErr != nil {
				if processErr == nil || response.OK {
					t.Fatalf("Go rejected %s (%v), Rust accepted (%s)", source, goErr, out)
				}
				return
			}
			if processErr != nil || !response.OK {
				t.Fatalf("Go accepted %s, Rust rejected: %v (%s)", source, processErr, out)
			}
			if response.Facts.Vertices != expected.vertices ||
				response.Facts.Triangles != expected.triangles ||
				response.Facts.SliceCaches != expected.sliceCaches ||
				response.Facts.PrinterModel != expected.printerModel {
				t.Fatalf("Go facts %+v differ from Rust %+v", expected, response.Facts)
			}
		})
	}
}
