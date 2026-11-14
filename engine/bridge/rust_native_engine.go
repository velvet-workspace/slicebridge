package bridge

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// convertWithNativeRust is an opt-in *application* path. Go resolves the
// approved installed machine, captures the source, and independently audits
// the published archive. Rust alone plans slots/materials/process settings and
// writes the converted 3MF; buildPlan, sourceSlotPlan, preservedProfile and
// converter.Convert are intentionally NOT called here.
func convertWithNativeRust(ctx context.Context, source, destination string, resolve profileResolver) (map[string]any, error) {
	src, dst, err := resolvePaths(source, destination)
	if err != nil {
		return nil, err
	}
	before, err := inspectPackage(src)
	if err != nil {
		return nil, fail("SOURCE_INVALID", "%v", err)
	}
	sourceNozzle, err := detectSourceNozzle(src)
	if err != nil {
		return nil, fail("SOURCE_NOZZLE_INVALID", "%v", err)
	}
	resolved, err := resolve(sourceNozzle)
	if err != nil {
		return nil, err
	}
	if err := validateResolved(resolved); err != nil {
		return nil, err
	}
	// Native Rust plans only from complete verified installed OEM snapshots.
	// Installed Flash Studio Flashforge Orca-family profiles may use the
	// same planner after Rust checks their independent identity/tool guards.
	// Other Orca forks can require special compatibility transformations not
	// yet ported to Rust and remain fail-closed; Creator 5 Pro is excluded.
	if resolved.bundle == nil {
		return nil, fail("TARGET_PROFILE_REQUIRED", "Rust-native requires a verified installed OEM bundle; builtin template is not accepted")
	}
	orcaPilot := resolved.Kind == ProfileInstalledOrca &&
		resolved.orca != nil &&
		resolved.orca.Slicer == "Flash Studio Desktop" &&
		strings.HasPrefix(resolved.ID, "orca-flashforge-") &&
		!strings.Contains(strings.ToLower(resolved.ExpectedModel), "creator 5")
	if resolved.Kind != ProfileInstalledVendor && !orcaPilot {
		return nil, fail("RUST_NATIVE_UNSUPPORTED", "Rust-native OEM compatibility not yet validated for %s", resolved.ID)
	}
	if err := resolved.base.Validate(); err != nil {
		return nil, fail("TARGET_PROFILE_INVALID", "%v", err)
	}
	inspection, err := converter.Inspect(src)
	if err != nil {
		return nil, fail("SOURCE_INVALID", "cannot read source filament declarations: %v", err)
	}
	if len(inspection.Filaments) == 0 {
		return nil, fail("NO_FILAMENTS", "source contains no usable filament data")
	}
	if err := validateSourceMaterialReferences(src, inspection.Filaments); err != nil {
		return nil, err
	}
	sourceSettings, err := readRustSourceSettings(src)
	if err != nil {
		return nil, fail("SOURCE_INVALID", "%v", err)
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_ENGINE_BIN")
	if binary == "" || !filepath.IsAbs(binary) {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native requires an explicit absolute SLICEBRIDGE_RUST_ENGINE_BIN")
	}
	info, err := os.Stat(binary)
	if err != nil || !info.Mode().IsRegular() {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native executable is not a regular local file: %s", binary)
	}
	outDir := filepath.Dir(dst)
	if st, err := os.Stat(outDir); err != nil || !st.IsDir() {
		return nil, fail("OUTPUT_INVALID", "output directory does not exist: %s", outDir)
	}
	if _, err := os.Lstat(dst); err == nil {
		return nil, fail("OUTPUT_EXISTS", "output already exists: %s", dst)
	} else if !os.IsNotExist(err) {
		return nil, fail("OUTPUT_INVALID", "%v", err)
	}
	// Deliberately omit Go mapping, process-preservation policy and conversion
	// plan. This request supplies only the independently verified OEM bundle,
	// literal source declarations and an immutable source settings snapshot.
	bundle := any(resolved.bundle)
	if orcaPilot {
		// Preserve the independently verified physical-tool count from the
		// installed Orca bundle; do not derive it from source color slots.
		bundle = resolved.orca
	}
	payload, err := json.Marshal(map[string]any{
		"bundle":                  bundle,
		"filaments":               inspection.Filaments,
		"source_settings":         sourceSettings,
		"requested_nozzle":        resolved.Nozzle,
		"include_receipt_details": true,
	})
	if err != nil || len(payload) > 8<<20 {
		return nil, fail("RUST_ENGINE_FAILED", "cannot encode bounded Rust-native request: %v", err)
	}
	jobDir, err := os.MkdirTemp(outDir, ".bridge-rust-native-job-*")
	if err != nil {
		return nil, fail("OUTPUT_INVALID", "cannot create private Rust-native job: %v", err)
	}
	defer os.RemoveAll(jobDir)
	requestPath := filepath.Join(jobDir, "verified-request.json")
	if err := os.WriteFile(requestPath, payload, 0600); err != nil {
		return nil, fail("RUST_ENGINE_FAILED", "cannot stage Rust-native request: %v", err)
	}
	output := filepath.Join(jobDir, "converted.3mf")
	if ctx == nil {
		ctx = context.Background()
	}
	bounded, cancel := context.WithTimeout(ctx, 120*time.Second)
	defer cancel()
	command := exec.CommandContext(bounded, binary, "--convert-native-archive", requestPath, src, output)
	command.WaitDelay = 2 * time.Second
	stdout := &cappedRustTranscript{limit: 64 << 10}
	stderr := &cappedRustTranscript{limit: 64 << 10}
	command.Stdout, command.Stderr = stdout, stderr
	runErr := command.Run()
	if bounded.Err() != nil {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native conversion canceled or timed out: %v", bounded.Err())
	}
	if stdout.truncated || stderr.truncated {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native receipt or diagnostic exceeds 64 KiB")
	}
	if runErr != nil {
		// One-shot Rust reports bounded machine-readable validation errors on
		// stdout even when exiting nonzero. Surface those details through the
		// existing app failure code rather than hiding the actual OEM/nozzle
		// mismatch behind an empty stderr. Never reinterpret it as success.
		var rejection struct {
			OK    bool   `json:"ok"`
			Error string `json:"error"`
		}
		if json.Unmarshal(stdout.buffer.Bytes(), &rejection) == nil && !rejection.OK && rejection.Error != "" {
			return nil, fail("RUST_ENGINE_FAILED", "Rust-native conversion refused input: %.512s", rejection.Error)
		}
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native conversion refused input: %v (stderr: %.512s)", runErr, stderr.buffer.String())
	}
	var receipt struct {
		OK           bool
		Experimental bool
		Removed      int
		Planner      string
		Mapping      map[string]int
		Slots        []converter.Slot
		Preserved    []string
	}
	if err := json.Unmarshal(stdout.buffer.Bytes(), &receipt); err != nil {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native invalid conversion receipt: %v", err)
	}
	if !receipt.OK || !receipt.Experimental || receipt.Planner != "rust-native" ||
		receipt.Removed < 0 || len(receipt.Slots) != len(inspection.Filaments) ||
		len(receipt.Mapping) != len(inspection.Filaments) {
		return nil, fail("RUST_ENGINE_FAILED", "Rust-native returned incomplete or inconsistent receipt")
	}
	mapping := make(map[int]int, len(receipt.Mapping))
	for key, slot := range receipt.Mapping {
		from, err := strconv.Atoi(key)
		if err != nil || from < 1 || slot < 1 || slot > len(receipt.Slots) {
			return nil, fail("RUST_ENGINE_FAILED", "Rust-native returned invalid slot mapping")
		}
		mapping[from] = slot
	}
	after, err := inspectPackage(output)
	if err != nil {
		return nil, fail("OUTPUT_VALIDATION_FAILED", "%v", err)
	}
	if before.vertices != after.vertices || before.triangles != after.triangles || after.triangles == 0 {
		return nil, fail("GEOMETRY_CHANGED", "mesh triangle/vertex counts differ: input %d/%d, output %d/%d",
			before.vertices, before.triangles, after.vertices, after.triangles)
	}
	if after.sliceCaches != 0 {
		return nil, fail("STALE_GCODE", "converted project still contains sliced machine code")
	}
	if after.printerModel != resolved.ExpectedModel {
		return nil, fail("TARGET_IDENTITY_INVALID", "converted project has unexpected printer model %q (expected %q)",
			after.printerModel, resolved.ExpectedModel)
	}
	// Audit the actual Rust project settings, not a profile rebuilt by Go.
	finalSettings, err := readRustSourceSettings(output)
	if err != nil {
		return nil, fail("OUTPUT_VALIDATION_FAILED", "Rust-native project settings: %v", err)
	}
	primeTowerReview := len(receipt.Slots) > 1 && primeTowerProfileEnabled(finalSettings)
	var towerPositions towerPlatePositionAudit
	if primeTowerReview {
		towerPositions = auditTargetTowerPlatePositions(output)
	}
	// Exclusive, same-filesystem publication. Never replace the source, an
	// existing output, or a path created concurrently by another conversion.
	if err := os.Link(output, dst); err != nil {
		if os.IsExist(err) {
			return nil, fail("OUTPUT_EXISTS", "output already exists: %s", dst)
		}
		return nil, fail("OUTPUT_PUBLISH_FAILED", "cannot publish without overwrite: %v", err)
	}
	warnings := []string{
		fmt.Sprintf("Open this project in the target slicer for %s and re-slice before printing.", resolved.ExpectedModel),
		"Review material slots, supports, nozzle and user print parameters; printer-specific firmware scripts come from the installed target profile.",
	}
	if len(receipt.Slots) > resolved.PhysicalTools {
		warnings = append(warnings, fmt.Sprintf(
			"All %d original color/material slots remain separate virtual project filaments. The target profile reports %s; this does not establish filament feeder capacity. Verify material-changing hardware and each slot's assignment in the native slicer. Do not combine or discard colors unless explicitly intended.",
			len(receipt.Slots), physicalToolPhrase(resolved.PhysicalTools)))
	}
	if len(receipt.Slots) > 1 {
		warnings = append(warnings,
			"Multi-color tool use is not verified by this 3MF conversion. Check the native slicer layer preview, physical material assignments and intended tool changes in sliced G-code before printing.")
	}
	if primeTowerReview {
		warnings = append(warnings,
			"Target prime/wipe tower position and clearance are not verified. Check for collision with the model, brim and excluded bed areas in the actual target slicer before printing; no tower coordinates are automatically moved.")
	}
	if primeTowerReview && towerPositions.incomplete() {
		warnings = append(warnings, fmt.Sprintf(
			"The converted 3MF contains %d plates but only %d X / %d Y prime tower position entries. Verify per-plate tower placement in the native slicer before slicing or printing; the bridge never invents missing coordinates.",
			towerPositions.plates, towerPositions.xEntries, towerPositions.yEntries))
	}
	if primeTowerReview && towerPositions.plates == 0 {
		warnings = append(warnings,
			"The converted 3MF plate count could not be determined from its native project metadata. Per-plate prime tower coordinates therefore cannot be audited; inspect each plate and tower clearance in the native slicer before printing.")
	}
	return map[string]any{
		"ok": true, "target": resolved.ID, "output": dst,
		"engine": "rust-native-3mf", "planner": receipt.Planner,
		"source_nozzle":                                sourceNozzle.Nozzle,
		"source_nozzle_source":                         sourceNozzle.Source,
		"source_nozzle_assumed":                        sourceNozzle.Assumed,
		"target_nozzle":                                resolved.Nozzle,
		"nozzle_remapped":                              sourceNozzle.Nozzle != resolved.Nozzle,
		"nozzle_selected_by_user":                      resolved.requestedNozzle() != "",
		"target_nozzle_origin":                         string(resolved.NozzleOrigin),
		"profile_kind":                                 string(resolved.Kind),
		"source_filament_slots":                        len(inspection.Filaments),
		"target_filament_slots_used":                   len(receipt.Slots),
		"virtual_filament_slots":                       len(receipt.Slots),
		"physical_toolheads":                           resolved.PhysicalTools,
		"user_slot_mapping_required":                   len(receipt.Slots) > resolved.PhysicalTools,
		"tool_usage_verification_required":             len(receipt.Slots) > 1,
		"prime_tower_clearance_review_required":        primeTowerReview,
		"prime_tower_plate_coordinate_review_required": primeTowerReview && towerPositions.incomplete(),
		"prime_tower_plate_position_audit_status":      towerPositions.status(primeTowerReview),
		"prime_tower_plates":                           towerPositions.plates,
		"prime_tower_coordinate_x_entries":             towerPositions.xEntries,
		"prime_tower_coordinate_y_entries":             towerPositions.yEntries,
		"source_to_target_slot_map":                    mapping,
		"geometry_vertices":                            after.vertices,
		"geometry_triangles":                           after.triangles,
		"removed_slice_cache_entries":                  receipt.Removed,
		"preserved_print_keys":                         receipt.Preserved,
		"warnings":                                     warnings,
	}, nil
}
