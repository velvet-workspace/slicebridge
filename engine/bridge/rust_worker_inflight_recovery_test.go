package bridge

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// An opt-in, isolated destructive-to-WORKER-ONLY recovery test. SIGKILL
// cannot trigger Rust Drop cleanup; always use a disposable directory.
// No real printer, user profile, installation or output destination is touched.
func TestRustWorkerInFlightHardKillAndRestart(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	corpus := os.Getenv("SLICEBRIDGE_STRESS_CORPUS_DIR")
	if rust == "" || corpus == "" {
		t.Skip("requires explicit Rust binary and synthetic stress corpus")
	}
	source := filepath.Join(corpus, "synthetic-deflate-240000.3mf")
	original, err := os.ReadFile(source)
	if err != nil {
		t.Skipf("generate optional 240000-tetrahedra synthetic corpus first: %v", err)
	}
	before := sha256.Sum256(original)
	info, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	slots, mapping, err := sourceSlotPlan(info.Filaments)
	if err != nil {
		t.Fatal(err)
	}
	settings, err := readRustSourceSettings(source)
	if err != nil {
		t.Fatal(err)
	}
	profile := printer.Builtin("snapmaker-u1")
	if len(slots) > profile.FilamentSlots {
		profile.FilamentSlots = len(slots)
	}
	if err := profile.Validate(); err != nil {
		t.Fatal(err)
	}
	m := make(map[string]int, len(mapping))
	for from, to := range mapping {
		m[strconv.Itoa(from)] = to
	}
	plan := map[string]any{
		"profile": profile, "slots": slots, "slot_count": profile.FilamentSlots,
		"source_settings": settings, "mapping": m,
		"supports": sourceSupportsForRust(settings), "preserved": []string{},
	}
	dir := t.TempDir()
	req, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	planFile := filepath.Join(dir, "plan.json")
	if err := os.WriteFile(planFile, req, 0600); err != nil {
		t.Fatal(err)
	}
	dest := filepath.Join(dir, "recovered.3mf")
	job, err := json.Marshal(map[string]any{
		"source": source, "plan_file": planFile, "destination": dest,
	})
	if err != nil {
		t.Fatal(err)
	}

	// Wait until Rust actually creates a private stage, not just until
	// the OS has spawned it. Kill only the process we launched ourselves.
	first := exec.Command(rust, "--convert-archive-stdio")
	stdin, err := first.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := first.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := first.Start(); err != nil {
		t.Fatal(err)
	}
	if _, err := stdin.Write(append(job, '\n')); err != nil {
		_ = first.Process.Kill()
		_ = first.Wait()
		t.Fatal(err)
	}
	deadline := time.Now().Add(8 * time.Second)
	observed := false
	for time.Now().Before(deadline) {
		files, _ := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial"))
		if len(files) > 0 {
			observed = true
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !observed {
		_ = first.Process.Kill()
		_ = stdin.Close()
		_ = stdout.Close()
		_ = first.Wait()
		t.Fatal("Rust worker did not expose an in-flight private stage in time")
	}
	if err := first.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	_ = stdin.Close()
	_ = stdout.Close()
	if err := first.Wait(); err == nil {
		t.Fatal("killed worker unexpectedly exited normally")
	}
	if _, err := os.Stat(dest); !os.IsNotExist(err) {
		t.Fatalf("killed in-flight Rust worker published output: %v", err)
	}

	// Deliberately leave any abandoned stage from the killed worker in
	// this isolated folder: restart must never mistake it for completed
	// output. A separate cleanup policy remains a production rollout gate.
	abandoned, _ := filepath.Glob(filepath.Join(dir, ".slicebridge-*.partial"))
	t.Logf("hard-stop left %d private stage files (isolated test dir; clean on test teardown)", len(abandoned))
	ctx, cancel := context.WithTimeout(context.Background(), 95*time.Second)
	defer cancel()
	resumed := exec.CommandContext(ctx, rust, "--convert-archive-stdio")
	in, err := resumed.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	out, err := resumed.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	resumed.Stderr = &stderr
	if err := resumed.Start(); err != nil {
		t.Fatal(err)
	}
	if _, err := in.Write(append(job, '\n')); err != nil {
		_ = resumed.Process.Kill()
		_ = resumed.Wait()
		t.Fatal(err)
	}
	line, err := bufio.NewReader(out).ReadBytes('\n')
	if err != nil {
		_ = resumed.Process.Kill()
		_ = resumed.Wait()
		t.Fatalf("restarted worker failed: %v; stderr=%s", err, stderr.String())
	}
	_ = in.Close()
	if err := resumed.Wait(); err != nil {
		t.Fatalf("restarted worker exit: %v; reply=%s; stderr=%s", err, line, stderr.String())
	}
	var reply struct {
		OK, Experimental bool
		Removed          int
		Error            string
	}
	if err := json.Unmarshal(line, &reply); err != nil || !reply.OK || !reply.Experimental {
		t.Fatalf("restart failed: %+v parse=%v", reply, err)
	}
	if _, err := inspectPackage(dest); err != nil {
		t.Fatalf("restarted output invalid 3MF: %v", err)
	}
	final, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if len(final) == 0 {
		t.Fatal("empty output after retry")
	}
	// A second launch targeting the completed path must refuse, leaving
	// the existing output untouched (at-most-once final publication).
	second := exec.CommandContext(ctx, rust, "--convert-archive-stdio")
	sin, err := second.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	sout, err := second.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := second.Start(); err != nil {
		t.Fatal(err)
	}
	if _, err := sin.Write(append(job, '\n')); err != nil {
		t.Fatal(err)
	}
	rejected, err := bufio.NewReader(sout).ReadBytes('\n')
	if err != nil {
		t.Fatal(err)
	}
	_ = sin.Close()
	if err := second.Wait(); err != nil {
		t.Fatal(err)
	}
	var refusal struct {
		OK    bool
		Error string
	}
	if err := json.Unmarshal(rejected, &refusal); err != nil || refusal.OK || refusal.Error == "" {
		t.Fatalf("restarted worker accepted duplicate output: %s", rejected)
	}
	unchanged, err := os.ReadFile(dest)
	if err != nil || !bytes.Equal(final, unchanged) {
		t.Fatal("rejected duplicate modified completed output")
	}
	after, err := os.ReadFile(source)
	if err != nil || sha256.Sum256(after) != before {
		t.Fatal("hard stop or restart mutated source")
	}
	t.Logf("PASS worker kill-in-flight, restart, safe duplicate refusal; output_bytes=%d source_SHA256=%x", len(final), before)
}
