package bridge

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// Exercises ONE Rust-native persistent process over two distinct approved
// source projects with an installed OEM snapshot, then rejects a duplicate
// destination without altering either source, profile or published result.
// No slicer UI, user settings or printer is touched.
func TestRustNativeWorkerRealOEMSequentialParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	mixed := os.Getenv("SLICEBRIDGE_MIXED_FIVE_SOURCE")
	if rust == "" || root == "" || mixed == "" {
		t.Skip("requires explicit Rust executable, approved OEM snapshots and mixed-five 3MF")
	}
	single := filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf")
	for _, target := range []string{"u1", "qidi"} {
		t.Run(target, func(t *testing.T) {
			oem := filepath.Join(root, "scratch-"+target+"-0.4-resolved.json")
			rawBundle, err := os.ReadFile(oem)
			if err != nil {
				t.Fatal(err)
			}
			var bundle map[string]any
			if err := json.Unmarshal(rawBundle, &bundle); err != nil {
				t.Fatal(err)
			}
			dir := t.TempDir()
			type job struct {
				source      string
				old         []byte
				goPath      string
				rustPath    string
				requestPath string
			}
			jobs := make([]job, 0, 2)
			for index, source := range []string{single, mixed} {
				original, err := os.ReadFile(source)
				if err != nil {
					t.Fatal(err)
				}
				inspect, err := converter.Inspect(source)
				if err != nil {
					t.Fatal(err)
				}
				settings, err := readRustSourceSettings(source)
				if err != nil {
					t.Fatal(err)
				}
				goOut := filepath.Join(dir, "go-"+string(rune('1'+index))+".3mf")
				rustOut := filepath.Join(dir, "rust-"+string(rune('1'+index))+".3mf")
				receipt, err := ConvertTarget(source, goOut, TargetRequest{
					Target: target, ProfileFile: oem, Nozzle: "0.4",
				})
				if err != nil || receipt["engine"] != "bl2std" {
					t.Fatalf("Go OEM baseline %s failed: %v %v", source, err, receipt)
				}
				input, err := json.Marshal(map[string]any{
					"bundle": bundle, "filaments": inspect.Filaments,
					"source_settings": settings, "requested_nozzle": "0.4",
				})
				if err != nil {
					t.Fatal(err)
				}
				req := filepath.Join(dir, "verified-request-"+string(rune('1'+index))+".json")
				if err := os.WriteFile(req, input, 0600); err != nil {
					t.Fatal(err)
				}
				jobs = append(jobs, job{source: source, old: original, goPath: goOut, rustPath: rustOut, requestPath: req})
			}
			ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
			defer cancel()
			command := exec.CommandContext(ctx, rust, "--convert-native-archive-stdio")
			input, err := command.StdinPipe()
			if err != nil {
				t.Fatal(err)
			}
			output, err := command.StdoutPipe()
			if err != nil {
				t.Fatal(err)
			}
			var stderr bytes.Buffer
			command.Stderr = &stderr
			if err := command.Start(); err != nil {
				t.Fatal(err)
			}
			reader := bufio.NewReader(output)
			send := func(j job) map[string]any {
				t.Helper()
				payload, err := json.Marshal(map[string]any{
					"request_file": j.requestPath, "source": j.source, "destination": j.rustPath,
				})
				if err != nil {
					t.Fatal(err)
				}
				if _, err := input.Write(append(payload, '\n')); err != nil {
					t.Fatal(err)
				}
				line, err := reader.ReadBytes('\n')
				if err != nil {
					t.Fatalf("native worker stopped before reply: %v %s", err, stderr.String())
				}
				var receipt map[string]any
				if err := json.Unmarshal(line, &receipt); err != nil {
					t.Fatalf("invalid worker reply: %v %s", err, line)
				}
				if _, leaked := receipt["project_settings"]; leaked {
					t.Fatal("native worker exposed machine settings")
				}
				return receipt
			}
			for _, j := range jobs {
				reply := send(j)
				if reply["ok"] != true || reply["experimental"] != true ||
					reply["planner"] != "rust-native" {
					t.Fatalf("native worker rejected verified OEM job: %v", reply)
				}
				compareConverted3MFParts(t, j.goPath, j.rustPath)
				after, err := os.ReadFile(j.source)
				if err != nil || !bytes.Equal(after, j.old) {
					t.Fatal("worker modified source")
				}
			}
			published, err := os.ReadFile(jobs[1].rustPath)
			if err != nil {
				t.Fatal(err)
			}
			refuse := send(jobs[1])
			why, _ := refuse["error"].(string)
			if refuse["ok"] != false || !strings.Contains(why, "duplicate destination") {
				t.Fatalf("worker accepted duplicate output: %v", refuse)
			}
			unchanged, err := os.ReadFile(jobs[1].rustPath)
			if err != nil || !bytes.Equal(published, unchanged) {
				t.Fatal("duplicate job mutated output")
			}
			if err := input.Close(); err != nil {
				t.Fatal(err)
			}
			if err := command.Wait(); err != nil {
				t.Fatalf("worker EOF failed: %v %s", err, stderr.String())
			}
			afterOEM, err := os.ReadFile(oem)
			if err != nil || !bytes.Equal(rawBundle, afterOEM) {
				t.Fatal("worker modified installed OEM snapshot")
			}
			if leftovers, err := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial")); err != nil || len(leftovers) != 0 {
				t.Fatalf("worker left scratch files: %v, err=%v", leftovers, err)
			}
			t.Logf("RUST_NATIVE_PERSISTENT_OEM_PARITY_PASS target=%s jobs=2 outputs=2 source_palette=one-and-five duplicate_refused=true", target)
		})
	}
}
