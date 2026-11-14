package bridge

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// rustEngineSelected is an explicit, developer-only opt-in. It never
// guesses a runtime from PATH and does not change the installed app default.
// Profile/nozzle/material policy remains under buildPlan and publishConversion.
func rustEngineSelected() bool {
	return os.Getenv("SLICEBRIDGE_CONVERSION_ENGINE") == "rust"
}

// readRustSourceSettings mirrors bl2std's source settings input. The production
// source inspector has already rejected duplicate ZIP entries and unsafe paths.
// No machine-owned settings are taken from the original source.
func readRustSourceSettings(source string) (map[string]any, error) {
	reader, err := zip.OpenReader(source)
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	settings := map[string]any{}
	for _, part := range reader.File {
		if part.Name != "Metadata/project_settings.config" {
			continue
		}
		const maxSettings = 8 << 20
		if part.UncompressedSize64 > maxSettings {
			return nil, fmt.Errorf("oversized source project settings")
		}
		stream, err := part.Open()
		if err != nil {
			return nil, err
		}
		data, readErr := io.ReadAll(io.LimitReader(stream, maxSettings+1))
		closeErr := stream.Close()
		if readErr != nil {
			return nil, readErr
		}
		if closeErr != nil {
			return nil, closeErr
		}
		if len(data) > maxSettings {
			return nil, fmt.Errorf("source project settings exceed 8 MiB")
		}
		if err := json.Unmarshal(data, &settings); err != nil {
			return nil, fmt.Errorf("source project settings: %w", err)
		}
		if settings == nil {
			return nil, fmt.Errorf("source project settings must be an object")
		}
		break
	}
	return settings, nil
}

// sourceSupportsForRust exactly matches the existing bl2std SupportsAuto
// contract: a direct "1" or an explicit settings override containing the key.
func sourceSupportsForRust(settings map[string]any) bool {
	if raw, ok := settings["enable_support"].(string); ok && raw == "1" {
		return true
	}
	if differences, ok := settings["different_settings_to_system"].([]any); ok {
		for _, item := range differences {
			if value, ok := item.(string); ok && strings.Contains(value, "enable_support") {
				return true
			}
		}
	}
	return false
}

// rustExplicitPlan binds the *already validated* resolved target and original
// project metadata to the experimental Rust ZIP assembler. It never uses a
// synthetic or fallback profile, and no unsafe guessed material assignments.
func rustExplicitPlan(plan conversionPlan, profile *printer.Profile, preserved []string) ([]byte, error) {
	sourceSettings, err := readRustSourceSettings(plan.source)
	if err != nil {
		return nil, err
	}
	mapping := make(map[string]int, len(plan.mapping))
	for from, to := range plan.mapping {
		mapping[strconv.Itoa(from)] = to
	}
	return json.Marshal(map[string]any{
		"profile":         profile,
		"slots":           plan.slots,
		"slot_count":      profile.FilamentSlots,
		"source_settings": sourceSettings,
		"mapping":         mapping,
		"supports":        sourceSupportsForRust(sourceSettings),
		"preserved":       preserved,
	})
}

// convertWithExperimentalRust runs the real Rust full-ZIP assembler in a
// bounded, child process. Its output goes to an unoccupied scratch path;
// the surrounding Go path still validates geometry, printer identity, stale
// G-code, native material plan, and atomic exclusive final publication.
// Failure is always returned: there is no silent Go fallback or empty success.
func convertWithExperimentalRust(plan conversionPlan, profile *printer.Profile, preserved []string, scratchOutput string) (int, error) {
	binary := os.Getenv("SLICEBRIDGE_RUST_ENGINE_BIN")
	if binary == "" || !filepath.IsAbs(binary) {
		return 0, fmt.Errorf("experimental Rust engine requires an explicit absolute SLICEBRIDGE_RUST_ENGINE_BIN")
	}
	info, err := os.Stat(binary)
	if err != nil || !info.Mode().IsRegular() {
		return 0, fmt.Errorf("experimental Rust binary is not a regular local file: %s", binary)
	}
	// Go's upstream converter validates the resolved profile before any
	// ZIP rewriting. The Rust path must enforce the same safety gate.
	if err := profile.Validate(); err != nil {
		return 0, fmt.Errorf("resolved target profile invalid: %w", err)
	}
	payload, err := rustExplicitPlan(plan, profile, preserved)
	if err != nil {
		return 0, err
	}
	// Both the plan and destination live inside the same authorized scratch
	// output directory. Plan files are private and promptly deleted.
	request, err := os.CreateTemp(filepath.Dir(scratchOutput), ".bridge-rust-plan-*.json")
	if err != nil {
		return 0, err
	}
	defer os.Remove(request.Name())
	if err := request.Chmod(0600); err != nil {
		request.Close()
		return 0, err
	}
	if _, err := request.Write(payload); err != nil {
		request.Close()
		return 0, err
	}
	if err := request.Sync(); err != nil {
		request.Close()
		return 0, err
	}
	if err := request.Close(); err != nil {
		return 0, err
	}
	if _, err := os.Stat(scratchOutput); !os.IsNotExist(err) {
		return 0, fmt.Errorf("Rust scratch destination is not vacant")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 120*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, "--convert-archive", request.Name(), plan.source, scratchOutput)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	stdout, err := cmd.Output()
	if ctx.Err() != nil {
		return 0, fmt.Errorf("experimental Rust conversion exceeded 120s limit: %w", ctx.Err())
	}
	if err != nil {
		return 0, fmt.Errorf("experimental Rust conversion refused input: %w (stderr: %.512s)", err, stderr.String())
	}
	var response struct {
		OK           bool   `json:"ok"`
		Removed      int    `json:"removed"`
		Experimental bool   `json:"experimental"`
		Error        string `json:"error"`
	}
	if err := json.Unmarshal(bytes.TrimSpace(stdout), &response); err != nil {
		return 0, fmt.Errorf("experimental Rust converter returned malformed receipt: %w", err)
	}
	if !response.OK || !response.Experimental || response.Removed < 0 {
		return 0, fmt.Errorf("experimental Rust conversion rejected plan: %s", response.Error)
	}
	return response.Removed, nil
}
