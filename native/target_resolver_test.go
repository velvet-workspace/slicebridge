package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
	"github.com/nocturney/slicebridge/native/presets"
)

// Everything below is synthetic: the repository's generated Bambu-style
// fixtures and invented machine profiles. No installed slicer, user profile or
// personal project is read, and no profile file is written outside t.TempDir().

const (
	testOrcaID     = "orca-flashforge-creator-5-pro-0-25-nozzle-abcdef0123"
	testOrcaOneID  = "orca-acme-mini-0-4-nozzle-0123456789"
	testOrcaSlicer = "Synthetic Orca"
)

func syntheticVendorBundle(target, model, nozzle string) *presets.Bundle {
	nozzles := []any{nozzle, nozzle, nozzle, nozzle}
	return &presets.Bundle{
		Target: target, Slicer: "Synthetic Vendor Slicer", Nozzle: nozzle,
		Profile: printer.Profile{
			Name: target, DisplayName: model, ModelID: model, FilamentSlots: 4,
			FilamentProfiles: map[string]string{"PLA": "Synthetic PLA"},
			ProjectSettings: map[string]any{
				"printer_model": model, "nozzle_diameter": nozzles,
				"machine_start_gcode": "G28 ; synthetic", "machine_end_gcode": "M104 S0 ; synthetic",
			},
		},
		MaterialSettings: map[string]map[string]any{"PLA": {
			"nozzle_temperature": []any{"210"}, "filament_flow_ratio": []any{"0.98"},
			"filament_max_volumetric_speed": []any{"18"},
			"filament_type":                 []any{"PLA"}, "filament_settings_id": []any{"Synthetic PLA"},
		}},
	}
}

func syntheticOrcaBundle(id, model, nozzle string, tools int) *presets.OrcaBundle {
	nozzles := make([]any, tools)
	for i := range nozzles {
		nozzles[i] = nozzle
	}
	return &presets.OrcaBundle{
		Target: id, Slicer: testOrcaSlicer, Nozzle: nozzle, PhysicalTools: tools,
		Profile: printer.Profile{
			Name: id, DisplayName: model, ModelID: model, FilamentSlots: tools,
			FilamentProfiles: map[string]string{"PLA": "Synthetic PLA"},
			ProjectSettings: map[string]any{
				"printer_model": model, "nozzle_diameter": nozzles,
				"machine_start_gcode": "G28 ; synthetic", "machine_end_gcode": "M104 S0 ; synthetic",
			},
		},
		MaterialSettings: map[string]map[string]any{"PLA": {
			"nozzle_temperature": []any{"210"}, "filament_flow_ratio": []any{"0.98"},
			"filament_max_volumetric_speed": []any{"18"},
			"filament_type":                 []any{"PLA"}, "filament_settings_id": []any{"Synthetic PLA"},
		}},
	}
}

// recordingLoaders serves synthetic profiles and records what was asked for, so
// tests can prove which nozzle the resolver requested from the installed slicer.
type recordingLoaders struct {
	vendorCalls []string
	orcaCalls   []string
	vendorErr   error
	orcaBundles map[string]*presets.OrcaBundle
}

func (r *recordingLoaders) loaders() targetLoaders {
	return targetLoaders{
		vendor: func(target, nozzle, slicer string, overrides map[string]string) (*presets.Bundle, error) {
			r.vendorCalls = append(r.vendorCalls, target+"@"+nozzle)
			if r.vendorErr != nil {
				return nil, r.vendorErr
			}
			models := map[string]string{
				"u1": "Snapmaker U1", "c5": "Flashforge Creator 5",
				"cc2": "Elegoo Centauri Carbon 2", "qidi": "X-Plus 5",
			}
			return syntheticVendorBundle(target, models[target], nozzle), nil
		},
		orca: func(id string) (*presets.OrcaBundle, error) {
			r.orcaCalls = append(r.orcaCalls, id)
			if bundle := r.orcaBundles[id]; bundle != nil {
				return bundle, nil
			}
			return nil, fmt.Errorf("installed target %s not found", id)
		},
	}
}

func TestResolverSavedQuarterMillimeterNozzleWinsOverSource(t *testing.T) {
	rec := &recordingLoaders{}
	resolved, refusal := resolveConversionTarget("c5", "0.4", PrinterSettings{Nozzle: "0.25", Slicer: "auto"}, rec.loaders())
	if refusal != nil {
		t.Fatal(refusal)
	}
	defer resolved.release()
	if got := strings.Join(rec.vendorCalls, ","); got != "c5@0.25" {
		t.Fatalf("installed profile requested for %q, want c5@0.25 (source is 0.4)", got)
	}
	if resolved.Request.Nozzle != "0.25" || resolved.Request.Target != "c5" || resolved.Prefix != "Creator5" {
		t.Fatalf("saved physical nozzle not carried to the conversion core: %+v", resolved.Request)
	}
	if _, err := os.Stat(resolved.Request.ProfileFile); err != nil {
		t.Fatalf("prepared profile missing: %v", err)
	}
}

func TestResolverSourceMatchingOnlyForAutoBuiltinAliases(t *testing.T) {
	rec := &recordingLoaders{}
	// Legacy automatic mode: the owner-approved 0.2 -> 0.25 Creator 5 remap.
	resolved, refusal := resolveConversionTarget("c5", "0.2", PrinterSettings{Nozzle: "auto", Slicer: "auto"}, rec.loaders())
	if refusal != nil {
		t.Fatal(refusal)
	}
	resolved.release()
	if strings.Join(rec.vendorCalls, ",") != "c5@0.25" || resolved.Request.Nozzle != "" {
		t.Fatalf("auto mode should follow the source and stay unselected: calls %v request %+v", rec.vendorCalls, resolved.Request)
	}
	// An explicit selection is never remapped, even when the legacy remap exists.
	rec = &recordingLoaders{}
	if _, refusal = resolveConversionTarget("c5", "0.4", PrinterSettings{Nozzle: "0.2"}, rec.loaders()); refusal == nil ||
		refusal.Code != "TARGET_NOZZLE_UNAVAILABLE" || len(rec.vendorCalls) != 0 {
		t.Fatalf("explicit 0.2 on the Creator 5 must fail closed without probing: %+v calls %v", refusal, rec.vendorCalls)
	}
	if _, refusal = resolveConversionTarget("u1", "0.4", PrinterSettings{Nozzle: "0.25"}, rec.loaders()); refusal == nil ||
		refusal.Code != "TARGET_NOZZLE_UNAVAILABLE" {
		t.Fatalf("nozzle the model does not offer was accepted: %+v", refusal)
	}
	if _, refusal = resolveConversionTarget("bogus", "0.4", PrinterSettings{}, rec.loaders()); refusal == nil ||
		refusal.Code != "TARGET_NOZZLE_UNAVAILABLE" {
		t.Fatalf("unknown alias was accepted: %+v", refusal)
	}
}

func TestResolverInstalledOrcaIgnoresSourceNozzleAndVendorLoader(t *testing.T) {
	rec := &recordingLoaders{orcaBundles: map[string]*presets.OrcaBundle{
		testOrcaID: syntheticOrcaBundle(testOrcaID, "Flashforge Creator 5 Pro", "0.25", 2),
	}}
	resolved, refusal := resolveConversionTarget(testOrcaID, "0.6", PrinterSettings{}, rec.loaders())
	if refusal != nil {
		t.Fatal(refusal)
	}
	if len(rec.vendorCalls) != 0 || strings.Join(rec.orcaCalls, ",") != testOrcaID {
		t.Fatalf("installed target must use only its own profile: vendor %v orca %v", rec.vendorCalls, rec.orcaCalls)
	}
	expectedPrefix := presets.InstalledOrcaOutputPrefix(testOrcaID, "Flashforge Creator 5 Pro", "", "0.25")
	if resolved.Slicer != testOrcaSlicer || resolved.Prefix != expectedPrefix ||
		!strings.HasSuffix(resolved.Prefix, "-0.25-SBabcdef0123") || resolved.Request.Nozzle != "" {
		t.Fatalf("unexpected resolution: %+v", resolved)
	}
	profile := resolved.Request.ProfileFile
	resolved.release()
	if _, err := os.Stat(profile); !os.IsNotExist(err) {
		t.Fatalf("temporary profile was not cleaned up: %v", err)
	}
}

func TestResolverRefusesMissingOrForeignInstalledProfiles(t *testing.T) {
	rec := &recordingLoaders{orcaBundles: map[string]*presets.OrcaBundle{
		testOrcaID: syntheticOrcaBundle(testOrcaOneID, "Acme Mini", "0.4", 1), // owned by a different ID
	}}
	for name, target := range map[string]string{"not installed": "orca-acme-gone-0-4-nozzle-ffffffffff", "foreign bundle": testOrcaID} {
		if _, refusal := resolveConversionTarget(target, "0.4", PrinterSettings{}, rec.loaders()); refusal == nil ||
			refusal.Code != "TARGET_PROFILE_INVALID" {
			t.Fatalf("%s: want TARGET_PROFILE_INVALID, got %+v", name, refusal)
		}
	}
	failing := &recordingLoaders{vendorErr: fmt.Errorf("incomplete installed profile")}
	if _, refusal := resolveConversionTarget("cc2", "0.4", PrinterSettings{}, failing.loaders()); refusal == nil ||
		refusal.Code != "TARGET_PROFILE_INVALID" {
		t.Fatalf("a missing vendor profile must fail closed for non-template targets: %+v", refusal)
	}
	// The U1 0.4 mm template stays the offline default and needs no profile file.
	resolved, refusal := resolveConversionTarget("u1", "0.4", PrinterSettings{Nozzle: "auto", Slicer: "auto"}, failing.loaders())
	if refusal != nil || resolved.Request.ProfileFile != "" || resolved.Slicer != "Built-in Snapmaker U1 template" {
		t.Fatalf("U1 template fallback broken: %+v %+v", resolved, refusal)
	}
}

func listDir(t *testing.T, dir string) []string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	names := []string{}
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	return names
}

// One five-color source, four different kinds of destination, one code path.
func TestMixedBuiltinAndInstalledTargetsConvertThroughOnePath(t *testing.T) {
	rec := &recordingLoaders{
		orcaBundles: map[string]*presets.OrcaBundle{
			testOrcaID:    syntheticOrcaBundle(testOrcaID, "Flashforge Creator 5 Pro", "0.25", 2),
			testOrcaOneID: syntheticOrcaBundle(testOrcaOneID, "Acme Mini", "0.4", 1),
		},
	}
	source := loadFixture(t, "bambu-five-colors.3mf")
	cases := []struct {
		target, nozzle, model string
		preference            PrinterSettings
		tools                 int
		kind, origin          string
	}{
		// u1 with no complete vendor install keeps the offline 0.4 template.
		{"u1", "0.4", "Snapmaker U1", PrinterSettings{Nozzle: "auto", Slicer: "auto"}, 4, "builtin_template", "source_matched"},
		{"c5", "0.4", "Flashforge Creator 5", PrinterSettings{Nozzle: "0.25", Slicer: "auto"}, 4, "installed_vendor_json", "selected"},
		{testOrcaID, "0.4", "Flashforge Creator 5 Pro", PrinterSettings{}, 2, "installed_orca_json", "profile"},
		{testOrcaOneID, "0.4", "Acme Mini", PrinterSettings{}, 1, "installed_orca_json", "profile"},
	}
	wantNozzle := map[string]string{"u1": "0.4", "c5": "0.25", testOrcaID: "0.25", testOrcaOneID: "0.4"}
	dir := filepath.Dir(source)
	for _, tc := range cases {
		t.Run(tc.target, func(t *testing.T) {
			loaders := rec.loaders()
			if tc.target == "u1" {
				loaders = (&recordingLoaders{vendorErr: fmt.Errorf("incomplete installed profile")}).loaders()
			}
			result := convertWithLoaders(source, tc.target, tc.nozzle, tc.preference, loaders)
			if !result.OK {
				t.Fatalf("%s: %s", result.Code, result.Error)
			}
			report := result.Report
			if report["target_nozzle"] != wantNozzle[tc.target] || report["source_nozzle"] != "0.4" {
				t.Fatalf("nozzle: %+v", report)
			}
			if report["target_nozzle_origin"] != tc.origin || report["profile_kind"] != tc.kind {
				t.Fatalf("provenance: %v / %v", report["target_nozzle_origin"], report["profile_kind"])
			}
			if report["physical_toolheads"] != tc.tools || report["virtual_filament_slots"] != 5 ||
				report["user_slot_mapping_required"] != (5 > tc.tools) {
				t.Fatalf("tool/slot accounting: %+v", report)
			}
			// Colors and slots survive in the written project itself.
			written, err := converter.Inspect(result.OutputPath)
			if err != nil {
				t.Fatal(err)
			}
			wantColors := []string{"#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FFFFFF"}
			if len(written.Filaments) != 5 {
				t.Fatalf("slots changed: %+v", written.Filaments)
			}
			for i, filament := range written.Filaments {
				if filament.Color != wantColors[i] || filament.Type != "PLA" {
					t.Fatalf("slot %d changed: %+v", i+1, filament)
				}
			}
			if report["target_filament_slots"] != 5 || result.Prefix == "" || !strings.HasPrefix(result.OutputName, result.Prefix+"-") {
				t.Fatalf("output naming/slot report: %q %q %+v", result.Prefix, result.OutputName, report)
			}
		})
	}
	// A temp profile must never survive a conversion.
	for _, name := range listDir(t, dir) {
		if strings.HasPrefix(name, ".bridge-") {
			t.Fatalf("conversion left temporary file %s", name)
		}
	}
}

func TestMismatchedSavedNozzleFailsClosedWithoutWritingOutput(t *testing.T) {
	rec := &recordingLoaders{orcaBundles: map[string]*presets.OrcaBundle{
		testOrcaID: syntheticOrcaBundle(testOrcaID, "Flashforge Creator 5 Pro", "0.25", 2),
	}}
	source := loadFixture(t, "bambu-five-colors.3mf")
	before := listDir(t, filepath.Dir(source))
	cases := map[string]struct {
		target     string
		preference PrinterSettings
		code       string
	}{
		"installed 0.25 machine, stale saved 0.4": {testOrcaID, PrinterSettings{Nozzle: "0.4"}, "TARGET_NOZZLE_MISMATCH"},
		"explicit 0.2 on the Creator 5":           {"c5", PrinterSettings{Nozzle: "0.2"}, "TARGET_NOZZLE_UNAVAILABLE"},
		"installed profile missing":               {"orca-acme-gone-0-4-nozzle-ffffffffff", PrinterSettings{}, "TARGET_PROFILE_INVALID"},
	}
	for name, tc := range cases {
		result := convertWithLoaders(source, tc.target, "0.4", tc.preference, rec.loaders())
		if result.OK || result.Code != tc.code {
			t.Fatalf("%s: want %s, got ok=%v %q %s", name, tc.code, result.OK, result.Code, result.Error)
		}
		if fmt.Sprint(listDir(t, filepath.Dir(source))) != fmt.Sprint(before) {
			t.Fatalf("%s: refusal left files behind: %v", name, listDir(t, filepath.Dir(source)))
		}
	}
}

func TestBuiltinAliasOutputPrefixesAreStable(t *testing.T) {
	want := map[string]string{"u1": "U1", "c5": "Creator5", "cc2": "CentauriCarbon2", "qidi": "XPlus5"}
	for alias, prefix := range want {
		if outputs[alias] != prefix {
			t.Fatalf("alias %s prefix %q, want %q", alias, outputs[alias], prefix)
		}
		if err := validateTargets([]string{alias}); err != nil {
			t.Fatalf("saved alias %s no longer accepted: %v", alias, err)
		}
	}
	if len(outputs) != len(want) {
		t.Fatalf("built-in alias set changed: %v", outputs)
	}
}
