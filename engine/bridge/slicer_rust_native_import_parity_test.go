package bridge

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

// TestRustNativeSlicerImportParity is an opt-in, read-only PrusaSlicer CLI
// import smoke test over scratch 3MF pairs emitted by
// TestRustFullArchiveExperimentalParity. It compares mesh-level import
// metrics for sources, Go output and Rust output, never slices G-code
// or contacts a physical printer.
//
// Required: SLICEBRIDGE_NATIVE_SLICER_EXE (PrusaSlicer executable),
// SLICEBRIDGE_SLICER_OUTPUT_DIR (18 existing scratch 3MF packages).
// PrusaSlicer --datadir is always isolated into t.TempDir().
func TestRustNativeSlicerImportParity(t *testing.T) {
	slicer := os.Getenv("SLICEBRIDGE_NATIVE_SLICER_EXE")
	scratch := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR")
	if slicer == "" || scratch == "" {
		t.Skip("set native slicer executable and generated scratch 3MF output directory")
	}
	if info, err := os.Stat(slicer); err != nil || info.IsDir() {
		t.Fatalf("PrusaSlicer executable unavailable: %q: %v", slicer, err)
	}
	cases := []struct {
		source, target int
		supports       bool
	}{
		{1, 4, false},
		{4, 4, false},
		{4, 4, true},
		{6, 6, false},
		{6, 4, false},
		{6, 4, true},
	}
	var paths []string
	digests := make(map[string][32]byte)
	for _, tc := range cases {
		suffix := fmt.Sprintf("%d-to-%d-support-%t", tc.source, tc.target, tc.supports)
		for _, kind := range []string{"source", "go", "rust"} {
			name := kind + "-" + suffix + ".3mf"
			path := filepath.Join(scratch, name)
			original, err := os.ReadFile(path)
			if err != nil {
				t.Fatalf("cannot read opt-in scratch 3MF %s: %v", name, err)
			}
			if len(original) == 0 || len(original) > 250<<20 {
				t.Fatalf("3MF fixture %s has unsafe file size", name)
			}
			digests[name] = sha256.Sum256(original)
			paths = append(paths, path)
		}
	}
	privateDir := t.TempDir()
	args := append([]string{"--info", "--datadir", privateDir}, paths...)
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	output, err := exec.CommandContext(ctx, slicer, args...).CombinedOutput()
	if ctx.Err() != nil {
		t.Fatalf("native slicer read timed out: %v", ctx.Err())
	}
	if err != nil {
		t.Fatalf("native slicer rejected scratch 3MF: %v\n%s", err, output)
	}
	records, err := parsePrusaSlicerInfo(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != len(paths) {
		t.Fatalf("native slicer parsed %d of %d input packages:\n%s", len(records), len(paths), output)
	}
	for _, tc := range cases {
		suffix := fmt.Sprintf("%d-to-%d-support-%t", tc.source, tc.target, tc.supports)
		source := records["source-"+suffix+".3mf"]
		goModel := records["go-"+suffix+".3mf"]
		rustModel := records["rust-"+suffix+".3mf"]
		if !reflect.DeepEqual(source, goModel) || !reflect.DeepEqual(goModel, rustModel) {
			t.Fatalf("%s differs in native PrusaSlicer mesh import: source=%v Go=%v Rust=%v", suffix, source, goModel, rustModel)
		}
		if source["number_of_facets"] == "" || source["size_x"] == "" ||
			source["size_y"] == "" || source["size_z"] == "" ||
			source["number_of_parts"] == "" || source["volume"] == "" {
			t.Fatalf("%s native slicer did not report geometry metrics: %v", suffix, source)
		}
	}
	for name, expected := range digests {
		data, err := os.ReadFile(filepath.Join(scratch, name))
		if err != nil || sha256.Sum256(data) != expected {
			t.Fatalf("native slicer changed scratch input %s", name)
		}
	}
	t.Logf("PrusaSlicer natively imported %d scratch packages; all %d Go/Rust/source geometry triplets match", len(paths), len(cases))
}

func parsePrusaSlicerInfo(data []byte) (map[string]map[string]string, error) {
	records := make(map[string]map[string]string)
	known := map[string]bool{
		"size_x": true, "size_y": true, "size_z": true,
		"min_x": true, "min_y": true, "min_z": true,
		"max_x": true, "max_y": true, "max_z": true,
		"number_of_facets": true, "manifold": true,
		"open_edges": true, "number_of_parts": true, "volume": true,
	}
	scanner := bufio.NewScanner(bytes.NewReader(data))
	var current string
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if strings.HasPrefix(line, "[") && strings.HasSuffix(line, ".3mf]") {
			current = strings.TrimSuffix(strings.TrimPrefix(line, "["), "]")
			if _, exists := records[current]; exists {
				return nil, fmt.Errorf("duplicate native slicer record %q", current)
			}
			records[current] = make(map[string]string)
			continue
		}
		if current == "" {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		key, value = strings.TrimSpace(key), strings.TrimSpace(value)
		if known[key] {
			records[current][key] = value
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("reading native slicer model info: %w", err)
	}
	return records, nil
}

func TestParsePrusaSlicerInfo(t *testing.T) {
	const sample = "[go-case.3mf]\nsize_x = 10.000000\nnumber_of_facets = 2\nvolume = 0.000000\n[rust-case.3mf]\nsize_x = 10.000000\nnumber_of_facets = 2\nvolume = 0.000000\n"
	parsed, err := parsePrusaSlicerInfo([]byte(sample))
	if err != nil || len(parsed) != 2 {
		t.Fatalf("cannot parse native slicer output: %v %v", parsed, err)
	}
	if !reflect.DeepEqual(parsed["go-case.3mf"], parsed["rust-case.3mf"]) {
		t.Fatalf("parser did not produce equal import metrics: %v", parsed)
	}
	if _, err := parsePrusaSlicerInfo([]byte("[a.3mf]\nsize_x=1\n[a.3mf]\nsize_x=2\n")); err == nil {
		t.Fatal("duplicate imported filename must be rejected")
	}
}

// TestRustNativeSlicerPinnedCorpusParity is the separate native-import gate
// for the eleven checked-in Bambu-style source fixtures and their Go/Rust
// converted outputs. A successful mesh import does NOT check real machine
// or material profiles, slicer-specific metadata, or printable topology.
func TestRustNativeSlicerPinnedCorpusParity(t *testing.T) {
	slicer := os.Getenv("SLICEBRIDGE_NATIVE_SLICER_EXE")
	scratch := os.Getenv("SLICEBRIDGE_SLICER_OUTPUT_DIR")
	if slicer == "" || scratch == "" {
		t.Skip("set a PrusaSlicer CLI and a directory containing the pinned corpus triplets")
	}
	baseNames, err := filepath.Glob(filepath.Join(scratch, "pinned-source-bambu-*.3mf"))
	if err != nil || len(baseNames) != 11 {
		t.Fatalf("expected 11 pinned source files in scratch: %v, count=%d", err, len(baseNames))
	}
	var paths []string
	digests := make(map[string][32]byte)
	for _, sourcePath := range baseNames {
		base := strings.TrimPrefix(filepath.Base(sourcePath), "pinned-source-")
		for _, kind := range []string{"source", "go", "rust"} {
			name := "pinned-" + kind + "-" + base
			filename := filepath.Join(scratch, name)
			content, err := os.ReadFile(filename)
			if err != nil || len(content) == 0 || len(content) > 250<<20 {
				t.Fatalf("pinned native slicer sample %q unavailable/unsafe: %v", name, err)
			}
			digests[name] = sha256.Sum256(content)
			paths = append(paths, filename)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	args := append([]string{"--info", "--datadir", t.TempDir()}, paths...)
	stdout, runErr := exec.CommandContext(ctx, slicer, args...).CombinedOutput()
	if ctx.Err() != nil {
		t.Fatalf("native slicer pinned corpus timed out: %v", ctx.Err())
	}
	if runErr != nil {
		t.Fatalf("native slicer rejected pinned corpus: %v\n%s", runErr, stdout)
	}
	metrics, err := parsePrusaSlicerInfo(stdout)
	if err != nil || len(metrics) != len(paths) {
		t.Fatalf("native slicer parsed %d/%d pinned outputs: %v\n%s", len(metrics), len(paths), err, stdout)
	}
	for _, sourcePath := range baseNames {
		base := strings.TrimPrefix(filepath.Base(sourcePath), "pinned-source-")
		src, goModel, rustModel := metrics["pinned-source-"+base], metrics["pinned-go-"+base], metrics["pinned-rust-"+base]
		if !reflect.DeepEqual(src, goModel) || !reflect.DeepEqual(goModel, rustModel) {
			t.Fatalf("pinned %s differs after native slicer import: source=%v Go=%v Rust=%v", base, src, goModel, rustModel)
		}
		if src["size_x"] == "" || src["number_of_facets"] == "" || src["volume"] == "" {
			t.Fatalf("missing native slicer geometry metrics for %s: %v", base, src)
		}
	}
	for name, sum := range digests {
		content, err := os.ReadFile(filepath.Join(scratch, name))
		if err != nil || sha256.Sum256(content) != sum {
			t.Fatalf("native slicer unexpectedly modified %s: %v", name, err)
		}
	}
	t.Logf("native PrusaSlicer import: %d independent pinned Bambu 3MF triplets matched exactly in geometry", len(baseNames))
}
