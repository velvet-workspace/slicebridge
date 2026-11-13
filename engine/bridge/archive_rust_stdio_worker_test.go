package bridge

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// TestRustStdioWorkerFullArchiveParity exercises a finite, opt-in local
// process reused for separate 3MF conversions. It must recover from a bad
// request, refuse duplicate output paths and produce two independent ZIPs
// with actual Go semantic parity. The test creates no persistent worker.
func TestRustStdioWorkerFullArchiveParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE")
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "source.3mf")
	fixture(t, source, 2)
	before, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	var sourceSettings map[string]any
	if err := json.Unmarshal(inspectZipEntries(t, source)["Metadata/project_settings.config"], &sourceSettings); err != nil {
		t.Fatal(err)
	}
	info, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	slots, mapping, err := sourceSlotPlan(info.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	profile := printer.Builtin("snapmaker-u1")
	if profile == nil {
		t.Fatal("missing target test profile")
	}
	goRaw := filepath.Join(dir, "go-raw.3mf")
	goOutput := filepath.Join(dir, "go-output.3mf")
	if _, err := converter.Convert(source, goRaw, converter.Options{
		Printer: profile, Slots: slots, Mapping: mapping, Supports: converter.SupportsOff,
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := stripSliceCache(goRaw, goOutput, nil); err != nil {
		t.Fatal(err)
	}
	slotMap := map[string]int{}
	for from, to := range mapping {
		slotMap[strconv.Itoa(from)] = to
	}
	plan := map[string]any{
		"profile": profile, "slots": slots, "slot_count": profile.FilamentSlots,
		"source_settings": sourceSettings, "mapping": slotMap,
		"supports": false, "preserved": []string{},
	}
	rawPlan, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	planFile := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(planFile, rawPlan, 0600); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	worker := exec.CommandContext(ctx, rust, "--convert-archive-stdio")
	in, err := worker.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	out, err := worker.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	worker.Stderr = &stderr
	if err := worker.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = in.Close()
		if worker.Process != nil {
			_ = worker.Process.Kill()
		}
		_ = worker.Wait()
	}()
	send := bufio.NewWriter(in)
	receive := bufio.NewReader(out)
	job := func(request map[string]any) (bool, string) {
		t.Helper()
		b, err := json.Marshal(request)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := send.Write(append(b, '\n')); err != nil {
			t.Fatal(err)
		}
		if err := send.Flush(); err != nil {
			t.Fatal(err)
		}
		line, err := receive.ReadBytes('\n')
		if err != nil {
			t.Fatalf("Rust stdio worker did not respond: %v stderr=%s", err, stderr.String())
		}
		var result struct {
			OK      bool   `json:"ok"`
			Removed int    `json:"removed"`
			Error   string `json:"error"`
		}
		if err := json.Unmarshal(line, &result); err != nil {
			t.Fatalf("Rust JSONL response invalid: %v %s", err, line)
		}
		return result.OK, fmt.Sprintf("removed=%d error=%s", result.Removed, result.Error)
	}
	if ok, detail := job(map[string]any{}); ok {
		t.Fatalf("incomplete job accepted: %s", detail)
	}
	for _, name := range []string{"first.3mf", "second.3mf"} {
		destination := filepath.Join(dir, name)
		request := map[string]any{
			"source": source, "destination": destination, "plan_file": planFile,
		}
		if ok, detail := job(request); !ok {
			t.Fatalf("Rust worker rejected valid request %s: %s", name, detail)
		}
		compareConverted3MFParts(t, goOutput, destination)
		if name == "first.3mf" {
			stable, err := os.ReadFile(destination)
			if err != nil {
				t.Fatal(err)
			}
			if ok, _ := job(request); ok {
				t.Fatal("persistent worker overwrote existing 3MF output")
			}
			after, err := os.ReadFile(destination)
			if err != nil || !bytes.Equal(stable, after) {
				t.Fatal("persistent worker altered a completed 3MF on rejected retry")
			}
		}
	}
	if err := in.Close(); err != nil {
		t.Fatal(err)
	}
	if err := worker.Wait(); err != nil {
		t.Fatalf("Rust worker did not exit cleanly on EOF: %v stderr=%s", err, stderr.String())
	}
	after, err := os.ReadFile(source)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("persistent worker modified input 3MF")
	}
	t.Log("two full 3MF ZIPs match Go, bad input and existing destination fail closed, worker exited on EOF")
}
