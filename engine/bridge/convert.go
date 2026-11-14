package bridge

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

type codedError struct {
	code  string
	cause error
}

func (e *codedError) Error() string { return e.cause.Error() }
func (e *codedError) Unwrap() error { return e.cause }
func fail(code string, format string, args ...any) error {
	return &codedError{code: code, cause: fmt.Errorf(format, args...)}
}
func ErrorCode(err error) string {
	var ce *codedError
	if errors.As(err, &ce) {
		return ce.code
	}
	return "CONVERSION_FAILED"
}

// maxVirtualFilaments caps hostile/accidental oversized palettes, not a
// physical hardware limit. The target slicer owns final spool/tool choices.
const maxVirtualFilaments = 64

// sourceSlotPlan represents every source filament ID as its own virtual slot,
// even if two source IDs have the same color and material. No automatic color
// folding, material substitution or spool decisions take place here.
func sourceSlotPlan(filaments []converter.Filament) ([]converter.Slot, map[int]int, error) {
	if len(filaments) == 0 || len(filaments) > maxVirtualFilaments {
		return nil, nil, fmt.Errorf("source has %d filament slots (allowed 1..%d)", len(filaments), maxVirtualFilaments)
	}
	slots := make([]converter.Slot, 0, len(filaments))
	mapping := make(map[int]int, len(filaments))
	for _, filament := range filaments {
		if filament.ID <= 0 {
			return nil, nil, fmt.Errorf("invalid source filament ID %d", filament.ID)
		}
		if _, duplicated := mapping[filament.ID]; duplicated {
			return nil, nil, fmt.Errorf("duplicate source filament ID %d", filament.ID)
		}
		if filament.Color == "" || filament.Type == "" {
			return nil, nil, fmt.Errorf("missing source color/material in filament %d", filament.ID)
		}
		mapping[filament.ID] = len(slots) + 1
		slots = append(slots, converter.Slot{
			Color:   filament.Color,
			Type:    filament.Type,
			Support: converter.IsSupportType(filament.Type, ""),
		})
	}
	return slots, mapping, nil
}

// Convert keeps the original validated U1 entrypoint for compatibility.
func Convert(source, destination, target string) (map[string]any, error) {
	return ConvertWithProfile(source, destination, target, "")
}

// ConvertWithProfile uses manufacturer slicer presets for C5/CC2/QIDI.
// A missing or mismatched installed profile fails closed.
func ConvertWithProfile(source, destination, target, profileFile string) (map[string]any, error) {
	return ConvertWithTargetNozzle(source, destination, target, profileFile, "")
}

// ConvertWithTargetNozzle allows the operator to choose the target's actual
// installed physical nozzle. The default retains historical source matching;
// an explicit target selection is validated against manufacturer sizes.
func ConvertWithTargetNozzle(source, destination, target, profileFile, requestedNozzle string) (map[string]any, error) {
	return ConvertTarget(source, destination, TargetRequest{Target: target, ProfileFile: profileFile, Nozzle: requestedNozzle})
}

// conversionPlan is the resolved input to the shared publish path. It is built
// only by buildPlan from a ResolvedTargetProfile (see resolved.go).
type conversionPlan struct {
	source, destination, target, expectedModel string
	base                                       *printer.Profile
	filaments                                  []converter.Filament
	slots                                      []converter.Slot
	mapping                                    map[int]int
	before                                     packageFacts
	physicalTools                              int
	sourceNozzle                               NozzleInfo
	targetNozzle, requestedNozzle              string
	profileKind                                ProfileKind
	nozzleOrigin                               NozzleOrigin
}

// resolvePaths applies the same path and extension rules to every engine.
func resolvePaths(source, destination string) (string, string, error) {
	src, err := filepath.Abs(source)
	if err != nil {
		return "", "", fail("SOURCE_INVALID", "%v", err)
	}
	dst, err := filepath.Abs(destination)
	if err != nil {
		return "", "", fail("OUTPUT_INVALID", "%v", err)
	}
	if strings.EqualFold(src, dst) {
		return "", "", fail("SOURCE_EQUALS_OUTPUT", "source and destination cannot be the same file")
	}
	if !strings.EqualFold(filepath.Ext(src), ".3mf") || !strings.EqualFold(filepath.Ext(dst), ".3mf") {
		return "", "", fail("EXTENSION_INVALID", "input and output must have .3mf extensions")
	}
	return src, dst, nil
}

// physicalToolPhrase keeps the historical four-tool warning text unchanged.
func physicalToolPhrase(count int) string {
	switch count {
	case 1:
		return "one physical tool"
	case 4:
		return "four physical tools"
	}
	return fmt.Sprintf("%d physical tools", count)
}

// vendorNativeImportWarnings distinguishes an archive's material metadata
// from what a particular native slicer will actually display on import.
// QIDIStudio can replace the printer/materials of an imported non-QIDI 3MF
// with local presets, so ZIP parity alone must not imply GUI material parity.
func vendorNativeImportWarnings(target string, slotCount int) []string {
	if target != "qidi" || slotCount < 1 {
		return nil
	}
	// A single-spool project still needs a native importer/material review:
	// QIDIStudio can replace imported non-native material settings even when
	// there is no material changer and the 3MF palette has only one slot.
	if slotCount == 1 {
		return []string{
			"QIDI X-Plus 5 has one physical print tool. QIDIStudio may replace printer and material settings when importing a 3MF it does not recognize as a native QIDIStudio project, including single-material projects. Verify the actual selected printer/nozzle, loaded physical filament, local material preset, temperature and flow settings before slicing or printing. A geometry-only import does not verify material compatibility; the bridge does not claim native QIDI GUI material acceptance. QIDIStudio's official release notes document automatic printer/material replacement for non-QIDIStudio 3MF projects. SliceBridge does not identify itself as QIDIStudio, so a correct on-disk material profile is not proof of native QIDIStudio import acceptance.",
		}
	}
	return []string{fmt.Sprintf(
		"QIDI X-Plus 5 has one physical print tool; multiple virtual filament slots do not provide automatic material switching. Actual automatic material switching depends on the specific user's installed feeder and printer configuration, not the physical nozzle count; verify the available material-switching path independently of 3MF import before printing. QIDIStudio may replace printer and material settings when importing a 3MF that it does not recognize as a native QIDIStudio project. This file contains %d separate virtual material slots, but the installed GUI may instead show one local filament preset. Verify that all %d material profiles, types and colours actually appear in QIDIStudio before slicing or printing; if not, treat the import as geometry-only. The bridge does not claim native QIDI multi-material GUI acceptance. QIDIStudio's official releases document automatic printer/material replacement when opening non-QIDIStudio 3MF projects. This SliceBridge output is not authored by QIDIStudio: the preserved archive palette can legitimately be ignored by that importer in favor of the active local filament.",
		slotCount, slotCount,
	)}
}

// publishConversion merges the resolved profile into the source project,
// verifies the sanitized output and publishes it without overwriting.
func publishConversion(plan conversionPlan) (map[string]any, error) {
	src, dst := plan.source, plan.destination
	base, slots, mapping := plan.base, plan.slots, plan.mapping
	before := plan.before
	// FilamentSlots controls the 3MF PROJECT palette length in bl2std. It is
	// NOT a claim that the target printer has this many physical extruders.
	if len(slots) > base.FilamentSlots {
		base.FilamentSlots = len(slots)
	}
	merged, preserved, err := preservedProfile(src, base)
	if err != nil {
		return nil, fail("SOURCE_INVALID", "%v", err)
	}
	for _, filament := range plan.filaments {
		if _, supported := merged.FilamentProfiles[filament.Type]; !supported {
			return nil, fail("MATERIAL_REVIEW_REQUIRED", "unsupported material %q in source slot %d; refusing silent PLA substitution", filament.Type, filament.ID)
		}
	}
	outDir := filepath.Dir(dst)
	if st, err := os.Stat(outDir); err != nil || !st.IsDir() {
		return nil, fail("OUTPUT_INVALID", "output directory does not exist: %s", outDir)
	}
	raw, err := os.CreateTemp(outDir, ".bridge-conversion-*.3mf")
	if err != nil {
		return nil, fail("OUTPUT_INVALID", "%v", err)
	}
	rawPath := raw.Name()
	raw.Close()
	defer os.Remove(rawPath)
	clean, err := os.CreateTemp(outDir, ".bridge-sanitized-*.3mf")
	if err != nil {
		return nil, fail("OUTPUT_INVALID", "%v", err)
	}
	cleanPath := clean.Name()
	clean.Close()
	defer os.Remove(cleanPath)

	_, err = converter.Convert(src, rawPath, converter.Options{
		Printer: merged, Slots: slots, Mapping: mapping, Supports: converter.SupportsAuto,
	})
	if err != nil {
		return nil, fail("UPSTREAM_CONVERSION_FAILED", "bl2std: %v", err)
	}
	removed, err := stripSliceCache(rawPath, cleanPath, preserved)
	if err != nil {
		return nil, fail("CACHE_SANITIZE_FAILED", "%v", err)
	}
	after, err := inspectPackage(cleanPath)
	if err != nil {
		return nil, fail("OUTPUT_VALIDATION_FAILED", "%v", err)
	}
	if before.vertices != after.vertices || before.triangles != after.triangles || after.triangles == 0 {
		return nil, fail("GEOMETRY_CHANGED", "mesh triangle/vertex counts differ: input %d/%d, output %d/%d", before.vertices, before.triangles, after.vertices, after.triangles)
	}
	if after.sliceCaches != 0 {
		return nil, fail("STALE_GCODE", "converted project still contains sliced machine code")
	}
	if after.printerModel != plan.expectedModel {
		return nil, fail("TARGET_IDENTITY_INVALID", "converted project has unexpected printer model %q (expected %q)", after.printerModel, plan.expectedModel)
	}
	primeTowerReview := len(slots) > 1 && primeTowerProfileEnabled(merged.ProjectSettings)
	var towerPositions towerPlatePositionAudit
	if primeTowerReview {
		towerPositions = auditTargetTowerPlatePositions(cleanPath)
	}
	// A hard link in the same directory publishes atomically without replacing
	// an existing output. This protects the source and all previous conversions.
	if err := os.Link(cleanPath, dst); err != nil {
		if os.IsExist(err) {
			return nil, fail("OUTPUT_EXISTS", "output already exists: %s", dst)
		}
		return nil, fail("OUTPUT_PUBLISH_FAILED", "cannot publish without overwrite: %v", err)
	}
	sort.Strings(preserved)
	warnings := []string{
		fmt.Sprintf("Open this project in the target slicer for %s and re-slice before printing.", plan.expectedModel),
		"Review material slots, supports, nozzle and user print parameters; printer-specific firmware scripts come from the installed target profile.",
	}
	if len(slots) > plan.physicalTools {
		warnings = append(warnings, fmt.Sprintf(
			"All %d original color/material slots remain separate virtual project filaments. The target profile reports %s; this does not establish filament feeder capacity. Verify material-changing hardware and each slot's assignment in the native slicer. Do not combine or discard colors unless explicitly intended.", len(slots), physicalToolPhrase(plan.physicalTools)))
	}
	if len(slots) > 1 {
		warnings = append(warnings,
			"Multi-color tool use is not verified by this 3MF conversion. Check the native slicer layer preview, physical material assignments and intended tool changes in sliced G-code before printing.")
	}
	warnings = append(warnings, vendorNativeImportWarnings(plan.target, len(slots))...)
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
		"ok":                                     true,
		"target":                                 plan.target,
		"output":                                 dst,
		"engine":                                 "bl2std",
		"source_nozzle":                          plan.sourceNozzle.Nozzle,
		"source_nozzle_source":                   plan.sourceNozzle.Source,
		"source_nozzle_assumed":                  plan.sourceNozzle.Assumed,
		"target_nozzle":                          plan.targetNozzle,
		"nozzle_remapped":                        plan.sourceNozzle.Nozzle != plan.targetNozzle,
		"nozzle_selected_by_user":                plan.requestedNozzle != "",
		"target_nozzle_origin":                   string(plan.nozzleOrigin),
		"profile_kind":                           string(plan.profileKind),
		"source_filament_slots":                  len(plan.filaments),
		"target_filament_slots_used":             len(slots),
		"virtual_filament_slots":                 len(slots),
		"physical_toolheads":                     plan.physicalTools,
		"user_slot_mapping_required":             len(slots) > plan.physicalTools,
		"tool_usage_verification_required":       len(slots) > 1,
		"native_slicer_material_review_required": plan.target == "qidi",
		"prime_tower_clearance_review_required":  primeTowerReview,
		"prime_tower_plate_coordinate_review_required": primeTowerReview && towerPositions.incomplete(),
		"prime_tower_plate_position_audit_status":      towerPositions.status(primeTowerReview),
		"prime_tower_plates":                           towerPositions.plates,
		"prime_tower_coordinate_x_entries":             towerPositions.xEntries,
		"prime_tower_coordinate_y_entries":             towerPositions.yEntries,
		"source_to_target_slot_map":                    mapping,
		"geometry_vertices":                            after.vertices,
		"geometry_triangles":                           after.triangles,
		"removed_slice_cache_entries":                  removed,
		"preserved_print_keys":                         preserved,
		"warnings":                                     warnings,
	}, nil
}
