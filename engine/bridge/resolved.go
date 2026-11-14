package bridge

import (
	"strings"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// ProfileKind records where a ResolvedTargetProfile came from. It describes the
// profile's provenance only; it never selects a conversion engine, because every
// kind runs through the same plan and publish path.
type ProfileKind string

const (
	// ProfileBuiltinTemplate is the field-validated 0.4 mm Snapmaker U1 template,
	// kept as an offline fallback for the historical default path.
	ProfileBuiltinTemplate ProfileKind = "builtin_template"
	// ProfileInstalledVendor is an installed manufacturer slicer profile for one
	// of the four stable built-in aliases (u1, c5, cc2, qidi).
	ProfileInstalledVendor ProfileKind = "installed_vendor_json"
	// ProfileInstalledOrca is one installed Orca-family machine preset.
	ProfileInstalledOrca ProfileKind = "installed_orca_json"
)

// NozzleOrigin states who decided the target's PHYSICAL nozzle. The source
// project's nozzle is only ever an input to the legacy "source matched" mode.
type NozzleOrigin string

const (
	// NozzleFromProfile: the installed machine profile fixes the nozzle.
	NozzleFromProfile NozzleOrigin = "profile"
	// NozzleSelected: the user's saved physical nozzle; the source is not consulted.
	NozzleSelected NozzleOrigin = "selected"
	// NozzleSourceMatched: legacy automatic mode for built-in aliases only. The
	// nozzle follows the source project and is reported as such.
	NozzleSourceMatched NozzleOrigin = "source_matched"
)

// TargetRequest is the single description of a conversion target accepted by
// ConvertTarget. The UI-facing choice is only Target (one physical printer);
// ProfileFile is the flattened installed bundle prepared by the native resolver
// and Nozzle is the user's saved physical nozzle, if any.
type TargetRequest struct {
	// Target is a stable built-in alias (u1, c5, cc2, qidi) or an installed
	// Orca target ID (orca-...).
	Target string
	// ProfileFile is empty only for the built-in 0.4 mm U1 template.
	ProfileFile string
	// Nozzle is the physical nozzle the user selected. Empty means legacy
	// source matching (built-in aliases only); installed Orca profiles fix their
	// own nozzle, and a non-empty value that disagrees is refused.
	Nozzle string
}

// ResolvedTargetProfile is the explicit contract between target resolution and
// the shared conversion core. Everything the core needs to know about the
// destination printer is here; the core never looks at the target name again.
// Only this package's loaders can construct one.
type ResolvedTargetProfile struct {
	ID            string
	Kind          ProfileKind
	ExpectedModel string
	// Nozzle is the target's physical nozzle (always a normalized size).
	Nozzle       string
	NozzleOrigin NozzleOrigin
	// PhysicalTools is the machine's real tool count. Source filament slots
	// beyond it stay virtual project filaments.
	PhysicalTools int

	base   *printer.Profile
	bundle *targetBundle // nil only for the built-in template
	orca   *orcaBundle   // installed Orca-only compatibility fixes, scoped by slicer/model
}

// requestedNozzle is the nozzle reported as "selected by user": anything that
// was not matched from the source project (a saved nozzle or a chosen machine).
func (r *ResolvedTargetProfile) requestedNozzle() string {
	if r.NozzleOrigin == NozzleSourceMatched {
		return ""
	}
	return r.Nozzle
}

// profileResolver turns the source project's nozzle into a resolved target. The
// source nozzle is passed ONLY so the legacy source-matched mode can use it.
type profileResolver func(source NozzleInfo) (*ResolvedTargetProfile, error)

// validateResolved is the one place that decides whether a resolved profile is
// complete enough to convert with. Anything essential that is missing fails
// closed before any file is written.
func validateResolved(r *ResolvedTargetProfile) error {
	if r == nil || r.base == nil {
		return fail("ENGINE_INVALID", "target base profile missing")
	}
	if r.ID == "" || r.ExpectedModel == "" {
		return fail("TARGET_IDENTITY_INVALID", "resolved target has no identity")
	}
	if r.PhysicalTools < 1 {
		return fail("TARGET_PROFILE_INVALID", "resolved target %s has no physical tool count", r.ID)
	}
	if normalized, err := normalizedNozzle(r.Nozzle); err != nil || normalized != r.Nozzle {
		return fail("TARGET_NOZZLE_UNAVAILABLE", "resolved target %s has no valid physical nozzle (%q)", r.ID, r.Nozzle)
	}
	if r.bundle != nil && r.bundle.Nozzle != r.Nozzle {
		return fail("TARGET_NOZZLE_MISMATCH",
			"requested %s nozzle %s mm but profile bundle is %s mm", r.ID, r.Nozzle, r.bundle.Nozzle)
	}
	return nil
}

// buildPlan is the single path from (source, destination, target resolver) to a
// conversionPlan. Built-in aliases and installed Orca profiles differ only in
// the resolver they pass.
func buildPlan(source, destination string, resolve profileResolver) (conversionPlan, error) {
	var plan conversionPlan
	src, dst, err := resolvePaths(source, destination)
	if err != nil {
		return plan, err
	}
	before, err := inspectPackage(src)
	if err != nil {
		return plan, fail("SOURCE_INVALID", "%v", err)
	}
	sourceNozzle, err := detectSourceNozzle(src)
	if err != nil {
		return plan, fail("SOURCE_NOZZLE_INVALID", "%v", err)
	}
	resolved, err := resolve(sourceNozzle)
	if err != nil {
		return plan, err
	}
	if err := validateResolved(resolved); err != nil {
		return plan, err
	}
	insp, err := converter.Inspect(src)
	if err != nil {
		return plan, fail("SOURCE_INVALID", "cannot read source filament assignments: %v", err)
	}
	if len(insp.Filaments) == 0 {
		return plan, fail("NO_FILAMENTS", "source contains no usable filament data")
	}
	slots, mapping, err := sourceSlotPlan(insp.Filaments)
	if err != nil {
		return plan, fail("SOURCE_FILAMENTS_INVALID", "%v", err)
	}
	if err := validateSourceMaterialReferences(src, insp.Filaments); err != nil {
		return plan, err
	}
	if resolved.bundle != nil {
		// The machine's physical tools remain unchanged. Extra slots are virtual
		// project filaments for manual slicer review.
		if len(slots) > resolved.bundle.Profile.FilamentSlots {
			resolved.bundle.Profile.FilamentSlots = len(slots)
		}
		if err = bindNativeFilamentSettings(resolved.bundle, insp.Filaments); err != nil {
			return plan, fail("MATERIAL_REVIEW_REQUIRED", "target %s: %v", resolved.ID, err)
		}
		// Flash Studio's verified four-tool Creator 5 output needs the same
		// support-flag alignment already applied to its installed Pro variant.
		// Scope only to the exact installed C5 vendor target, not other slicers.
		if resolved.Kind == ProfileInstalledVendor && resolved.ID == "c5" {
			if err = normalizeFlashStudioCreator5SupportFlags(resolved.bundle, len(insp.Filaments)); err != nil {
				return plan, fail("TARGET_PROFILE_INVALID", "target %s: %v", resolved.ID, err)
			}
		}
		// Keep all installed Orca-fork compatibility guards from published dev.24.
		// They operate on the resolved in-memory bundle, never a saved vendor file.
		if resolved.orca != nil {
			if err = normalizeInstalledOrcaCompatibility(resolved.orca, len(insp.Filaments)); err != nil {
				return plan, fail("TARGET_PROFILE_INVALID", "target %s: %v", resolved.ID, err)
			}
		}
	}
	return conversionPlan{
		source: src, destination: dst, target: resolved.ID,
		expectedModel: resolved.ExpectedModel,
		base:          resolved.base, filaments: insp.Filaments, slots: slots, mapping: mapping,
		before: before, physicalTools: resolved.PhysicalTools, sourceNozzle: sourceNozzle,
		targetNozzle: resolved.Nozzle, requestedNozzle: resolved.requestedNozzle(),
		profileKind: resolved.Kind, nozzleOrigin: resolved.NozzleOrigin,
	}, nil
}

// convertWith runs the one shared path: plan, then publish.
func convertWith(source, destination string, resolve profileResolver) (map[string]any, error) {
	plan, err := buildPlan(source, destination, resolve)
	if err != nil {
		return nil, err
	}
	return publishConversion(plan)
}

// ConvertTarget converts a project for one physical target. It is the only
// entry point the desktop app needs: built-in aliases and installed Orca IDs are
// distinguished solely by how their profile bundle is loaded and validated.
func ConvertTarget(source, destination string, req TargetRequest) (map[string]any, error) {
	if orcaTargetID.MatchString(req.Target) {
		return convertWith(source, destination, func(NozzleInfo) (*ResolvedTargetProfile, error) {
			return resolveInstalledOrca(req.ProfileFile, req.Target, req.Nozzle)
		})
	}
	if targetModels[strings.ToLower(req.Target)] == "" {
		return nil, fail("TARGET_NOT_VALIDATED", "unknown target %q", strings.ToLower(req.Target))
	}
	return convertWith(source, destination, func(src NozzleInfo) (*ResolvedTargetProfile, error) {
		return resolveBuiltinTarget(req.Target, req.ProfileFile, req.Nozzle, src.Nozzle)
	})
}

// resolveBuiltinTarget resolves one of the four stable aliases. A user-selected
// nozzle is validated against the manufacturer sizes and is never replaced by
// the source project's nozzle; only an empty selection follows the source.
func resolveBuiltinTarget(target, profileFile, requestedNozzle, sourceNozzle string) (*ResolvedTargetProfile, error) {
	target = strings.ToLower(target)
	model := targetModels[target]
	if model == "" {
		return nil, fail("TARGET_NOT_VALIDATED", "unknown target %q", target)
	}
	nozzle, origin := requestedNozzle, NozzleSelected
	if nozzle == "" {
		var err error
		if nozzle, err = targetNozzleForSource(target, sourceNozzle); err != nil {
			return nil, fail("TARGET_NOZZLE_UNAVAILABLE", "%v", err)
		}
		origin = NozzleSourceMatched
	} else if !validTargetNozzle(target, nozzle) {
		return nil, fail("TARGET_NOZZLE_UNAVAILABLE", "target %s does not support physical nozzle %q", target, requestedNozzle)
	}
	// Built-in aliases historically reported four physical tools for all
	// machines. X-Plus 5 has one print nozzle/tool; an optional QIDI BOX
	// expands filament feeding, NOT the number of physical print tools.
	// The installed OEM project's default four palette slots are virtual
	// project metadata and must not determine this count.
	physicalTools := 4
	if target == "qidi" {
		physicalTools = 1
	}
	resolved := &ResolvedTargetProfile{
		ID: target, ExpectedModel: model, Nozzle: nozzle, NozzleOrigin: origin, PhysicalTools: physicalTools,
	}
	if target == "u1" && nozzle == "0.4" && profileFile == "" {
		// Only use the legacy fallback when no complete installed vendor
		// profile was supplied. An installed profile must never be ignored.
		resolved.Kind = ProfileBuiltinTemplate
		resolved.base = printer.Builtin("snapmaker-u1")
		return resolved, nil
	}
	if profileFile == "" {
		return nil, fail("TARGET_PROFILE_REQUIRED", "installed %s %smm slicer nozzle profile is required", target, nozzle)
	}
	bundle, err := loadTargetBundle(profileFile, target)
	if err != nil {
		return nil, fail("TARGET_PROFILE_INVALID", "target %s installed slicer profile: %v", target, err)
	}
	resolved.Kind = ProfileInstalledVendor
	resolved.bundle = bundle
	resolved.base = &bundle.Profile
	return resolved, nil
}

// resolveInstalledOrca resolves one installed Orca-family machine. The profile
// fixes model, nozzle and tool count; a requested nozzle or target ID that
// disagrees with it is refused instead of being reconciled.
func resolveInstalledOrca(profileFile, expectedID, requestedNozzle string) (*ResolvedTargetProfile, error) {
	if profileFile == "" {
		return nil, fail("TARGET_PROFILE_REQUIRED", "installed profile bundle is required for %s", expectedID)
	}
	bundle, err := loadOrcaBundle(profileFile)
	if err != nil {
		return nil, fail("TARGET_PROFILE_INVALID", "installed profile: %v", err)
	}
	if expectedID != "" && bundle.Target != expectedID {
		return nil, fail("TARGET_PROFILE_INVALID", "installed profile bundle %s does not belong to requested target %s", bundle.Target, expectedID)
	}
	if requestedNozzle != "" && requestedNozzle != bundle.Nozzle {
		return nil, fail("TARGET_NOZZLE_MISMATCH",
			"requested %s nozzle %s mm but installed profile is %s mm", bundle.Target, requestedNozzle, bundle.Nozzle)
	}
	return &ResolvedTargetProfile{
		ID: bundle.Target, Kind: ProfileInstalledOrca, ExpectedModel: bundle.Profile.ModelID,
		Nozzle: bundle.Nozzle, NozzleOrigin: NozzleFromProfile, PhysicalTools: bundle.PhysicalTools,
		base: &bundle.Profile, bundle: &bundle.targetBundle, orca: bundle,
	}, nil
}
