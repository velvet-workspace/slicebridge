package bridge

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

const syntheticOrcaID = "orca-acme-mini-0-4-nozzle-0123456789"

// syntheticOrcaBundle mirrors what the native resolver writes for an installed
// single-nozzle machine. Tests mutate it to exercise each fail-closed branch.
func syntheticOrcaBundle(tools int) *orcaBundle {
	nozzles := make([]any, tools)
	for i := range nozzles {
		nozzles[i] = "0.4"
	}
	return &orcaBundle{
		targetBundle: targetBundle{
			Target: syntheticOrcaID, Slicer: "Synthetic Orca", Nozzle: "0.4",
			Profile: printer.Profile{
				Name: syntheticOrcaID, DisplayName: "Acme Mini", ModelID: "Acme Mini",
				FilamentSlots:    tools,
				FilamentProfiles: map[string]string{"PLA": "Acme PLA"},
				ProjectSettings: map[string]any{
					"printer_model":       "Acme Mini",
					"nozzle_diameter":     nozzles,
					"machine_start_gcode": "G28 ; installed",
					"machine_end_gcode":   "M104 S0 ; installed",
				},
			},
			MaterialSettings: map[string]map[string]any{
				"PLA": {
					"nozzle_temperature":            []any{"210"},
					"filament_flow_ratio":           []any{"0.98"},
					"filament_max_volumetric_speed": []any{"18"},
					"filament_type":                 []any{"PLA"},
					"filament_settings_id":          []any{"Acme PLA"},
				},
			},
		},
		PhysicalTools: tools,
	}
}

func writeOrcaBundle(t *testing.T, bundle *orcaBundle) string {
	t.Helper()
	data, err := json.Marshal(bundle)
	if err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(t.TempDir(), "orca-profile.json")
	if err := os.WriteFile(file, data, 0600); err != nil {
		t.Fatal(err)
	}
	return file
}

func TestOrcaBundleAcceptsStructurallyValidSingleToolSample(t *testing.T) {
	bundle, err := loadOrcaBundle(writeOrcaBundle(t, syntheticOrcaBundle(1)))
	if err != nil {
		t.Fatal(err)
	}
	if bundle.PhysicalTools != 1 || bundle.Profile.FilamentSlots != 1 {
		t.Fatalf("physical tools must come from the machine, got %d/%d", bundle.PhysicalTools, bundle.Profile.FilamentSlots)
	}
	if bundle.Profile.ModelID != "Acme Mini" || bundle.Nozzle != "0.4" {
		t.Fatalf("identity not preserved: model %q nozzle %q", bundle.Profile.ModelID, bundle.Nozzle)
	}
}

func TestOrcaBundleRejectsUnverifiedIdentityAndTools(t *testing.T) {
	cases := map[string]func(*orcaBundle){
		"tool count not backed by nozzle array": func(b *orcaBundle) {
			b.PhysicalTools = 2
			b.Profile.FilamentSlots = 2
		},
		"mixed nozzle diameters": func(b *orcaBundle) {
			b.PhysicalTools = 2
			b.Profile.FilamentSlots = 2
			b.Profile.ProjectSettings["nozzle_diameter"] = []any{"0.4", "0.6"}
		},
		"model already served by a built-in engine": func(b *orcaBundle) {
			b.Profile.DisplayName = "X-Plus 5"
			b.Profile.ModelID = "X-Plus 5"
			b.Profile.ProjectSettings["printer_model"] = "X-Plus 5"
		},
		"legacy target name claimed": func(b *orcaBundle) {
			b.Target = "qidi"
			b.Profile.Name = "qidi"
		},
		"missing PLA preset": func(b *orcaBundle) {
			b.Profile.FilamentProfiles = map[string]string{"PETG": "Acme PETG"}
		},
	}
	for name, mutate := range cases {
		bundle := syntheticOrcaBundle(1)
		mutate(bundle)
		if _, err := loadOrcaBundle(writeOrcaBundle(t, bundle)); err == nil {
			t.Fatalf("%s was accepted", name)
		}
	}
}

func TestFiveVirtualSlotsOnOneToolKeepEverySourceSlot(t *testing.T) {
	bundle, err := loadOrcaBundle(writeOrcaBundle(t, syntheticOrcaBundle(1)))
	if err != nil {
		t.Fatal(err)
	}
	filaments := []converter.Filament{
		{ID: 1, Type: "PLA", Color: "#FF0000"},
		// Identical color and material must stay a separate virtual slot.
		{ID: 2, Type: "PLA", Color: "#FF0000"},
		{ID: 3, Type: "PLA", Color: "#00FF00"},
		{ID: 4, Type: "PLA", Color: "#0000FF"},
		{ID: 5, Type: "PLA", Color: "#FFFFFF"},
	}
	slots, mapping, err := sourceSlotPlan(filaments)
	if err != nil {
		t.Fatal(err)
	}
	if len(slots) != 5 || len(mapping) != 5 || mapping[2] != 2 {
		t.Fatalf("source slots folded: %d slots, mapping %v", len(slots), mapping)
	}
	if slots[0].Color != slots[1].Color {
		t.Fatal("test precondition: first two slots should share a color")
	}
	bundle.Profile.FilamentSlots = len(slots)
	if err := bindNativeFilamentSettings(&bundle.targetBundle, filaments); err != nil {
		t.Fatal(err)
	}
	cfg := bundle.Profile.ProjectSettings
	if !reflect.DeepEqual(cfg["filament_type"], []any{"PLA", "PLA", "PLA", "PLA", "PLA"}) {
		t.Fatalf("virtual slot material list wrong: %v", cfg["filament_type"])
	}
	if got, _ := cfg["nozzle_temperature"].([]any); len(got) != 5 {
		t.Fatalf("per-slot temperatures should cover 5 virtual slots: %v", cfg["nozzle_temperature"])
	}
	if bundle.PhysicalTools != 1 {
		t.Fatalf("virtual slots must not change the physical tool count: %d", bundle.PhysicalTools)
	}
}

func TestPhysicalToolWarningWordingIsStable(t *testing.T) {
	if got := physicalToolPhrase(4); got != "four physical tools" {
		t.Fatalf("legacy four-tool wording changed: %q", got)
	}
	if got := physicalToolPhrase(1); got != "one physical tool" {
		t.Fatalf("single-tool wording wrong: %q", got)
	}
	if got := physicalToolPhrase(2); got != "2 physical tools" {
		t.Fatalf("two-tool wording wrong: %q", got)
	}
}

// Creality Print 7.3 rejects output when support flags are shorter than the
// project palette, even when the same installed PLA preset backs every slot.
func TestCrealitySupportFlagsMatchVirtualFilamentSlots(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Slicer = "Creality Print"
	b.Profile.ModelID = "Creality K2 Plus"
	b.Profile.ProjectSettings["filament_is_support"] = []any{"0"}
	if err := normalizeCrealitySupportFlags(b, 2); err != nil {
		t.Fatal(err)
	}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, []any{"0", "0"}) {
		t.Fatalf("multi-colour support flags misaligned: %v", got)
	}
	if got := b.Profile.ProjectSettings["nozzle_diameter"]; !reflect.DeepEqual(got, []any{"0.4"}) {
		t.Fatalf("physical nozzle was modified: %v", got)
	}
	if b.PhysicalTools != 1 {
		t.Fatalf("physical tool count changed to %d", b.PhysicalTools)
	}
}

func TestCrealityFlagNormalizationDoesNotTouchOtherOrcaForks(t *testing.T) {
	for _, tc := range []struct{ slicer, model string }{
		{"QIDIStudio", "QIDI X-Plus 5"},
		{"OrcaSlicer", "Creality K2 Plus"},
		{"Creality Print", "Other Vendor"},
	} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID = tc.slicer, tc.model
		b.Profile.ProjectSettings["filament_is_support"] = []any{"0"}
		if err := normalizeCrealitySupportFlags(b, 3); err != nil {
			t.Fatal(err)
		}
		if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, []any{"0"}) {
			t.Fatalf("slicer %s / model %s mutated: %v", tc.slicer, tc.model, got)
		}
	}
}

func TestCrealitySupportFlagsRefuseAmbiguousValues(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
	b.Profile.ProjectSettings["filament_is_support"] = []any{"0", "1"}
	if err := normalizeCrealitySupportFlags(b, 3); err == nil {
		t.Fatal("ambiguous per-filament flags must not be silently remapped")
	}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, []any{"0", "1"}) {
		t.Fatalf("ambiguous flags changed: %v", got)
	}
}

// The installed Creality profile may omit this optional setting; the upstream
// converter's single-slot default is "0". Replicating it is necessary for
// Creality Print's multi-filament 3MF validator.
func TestCrealityMissingSupportFlagsUseInstalledDefault(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
	if err := normalizeCrealitySupportFlags(b, 2); err != nil {
		t.Fatal(err)
	}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, []any{"0", "0"}) {
		t.Fatalf("missing support flags should become explicit defaults: %v", got)
	}
}

// Current Creality 7.3 vendor profiles store this value as a scalar string,
// while their CLI requires one value per virtual filament.
func TestCrealityScalarSupportFlagIsExpanded(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
	b.Profile.ProjectSettings["filament_is_support"] = "0"
	if err := normalizeCrealitySupportFlags(b, 2); err != nil {
		t.Fatal(err)
	}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, []any{"0", "0"}) {
		t.Fatalf("scalar support flag not normalized: %v", got)
	}
}

// A real 3-color Bambu project only sliced T0 in Creality Print 7.3 when
// the installed scalar filament diameter was not expanded. A diagnostic
// copy with 3 diameter entries produced T0/T1/T2 and distinct material usage.
func TestCrealityVirtualFilamentDiameterMatchesPreservedPalette(t *testing.T) {
	for _, tc := range []struct{ slots int }{{2}, {3}, {4}} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
		b.Profile.ProjectSettings["filament_diameter"] = "1.75"
		if err := normalizeCrealityFilamentDiameters(b, tc.slots); err != nil {
			t.Fatal(err)
		}
		want := make([]any, tc.slots)
		for i := range want {
			want[i] = "1.75"
		}
		if got := b.Profile.ProjectSettings["filament_diameter"]; !reflect.DeepEqual(got, want) {
			t.Fatalf("slots=%d diameter array=%v", tc.slots, got)
		}
		if got := b.Profile.ProjectSettings["nozzle_diameter"]; !reflect.DeepEqual(got, []any{"0.4"}) {
			t.Fatal("physical nozzle changed")
		}
	}
}

func TestCrealityDiameterNormalizationKeepsOtherOrcaUntouched(t *testing.T) {
	for _, tc := range []struct{ slicer, model string }{
		{"OrcaSlicer", "Creality K2 Plus"},
		{"QIDIStudio", "QIDI X-Plus 5"},
		{"Anycubic Slicer Next", "Anycubic Kobra S1"},
		{"Creality Print", "Other Vendor"},
	} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID = tc.slicer, tc.model
		b.Profile.ProjectSettings["filament_diameter"] = "1.75"
		if err := normalizeCrealityFilamentDiameters(b, 3); err != nil {
			t.Fatal(err)
		}
		if got := b.Profile.ProjectSettings["filament_diameter"]; got != "1.75" {
			t.Fatalf("%s / %s diameter changed: %v", tc.slicer, tc.model, got)
		}
	}
}

func TestCrealityDiameterNormalizationRefusesAmbiguousValues(t *testing.T) {
	for _, values := range []any{nil, []any{}, []any{"1.75", "2.85"}, ""} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
		if values != nil {
			b.Profile.ProjectSettings["filament_diameter"] = values
		}
		if err := normalizeCrealityFilamentDiameters(b, 3); err == nil {
			t.Fatalf("ambiguous diameter %v was accepted", values)
		}
	}
}

func TestCrealityDiameterArrayAlreadyCompleteIsPreserved(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Slicer, b.Profile.ModelID = "Creality Print", "Creality K2 Plus"
	want := []any{"1.75", "1.75", "1.75"}
	b.Profile.ProjectSettings["filament_diameter"] = want
	if err := normalizeCrealityFilamentDiameters(b, 3); err != nil {
		t.Fatal(err)
	}
	if got := b.Profile.ProjectSettings["filament_diameter"]; !reflect.DeepEqual(got, want) {
		t.Fatalf("explicitly sized diameters altered: %v", got)
	}
}

// A four-tool Creator 5 Pro project previously failed Flash Studio 2.4.2 CLI
// (error -5: filament_is_support count 1 vs filament_colour size 4).
// Repeating its missing default "0" in a disposable project made genuine
// vendor 0.25, 0.6 and 0.8 nozzle presets slice successfully.
func TestFlashforgeCreator5ProMissingSupportFlagUsesFourNativeSlots(t *testing.T) {
	b := syntheticOrcaBundle(4)
	b.Slicer, b.Profile.ModelID = "Flash Studio", "Flashforge Creator 5 Pro"
	b.Nozzle = "0.25"
	b.Profile.ProjectSettings["nozzle_diameter"] = []any{"0.25", "0.25", "0.25", "0.25"}
	if err := normalizeFlashforgeCreator5ProSupportFlags(b, 1); err != nil {
		t.Fatal(err)
	}
	want := []any{"0", "0", "0", "0"}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, want) {
		t.Fatalf("missing four tool flags: %v", got)
	}
	if got := b.Profile.ProjectSettings["nozzle_diameter"]; !reflect.DeepEqual(got, []any{"0.25", "0.25", "0.25", "0.25"}) {
		t.Fatalf("manufacturer nozzle diameters modified: %v", got)
	}
	if b.PhysicalTools != 4 || b.Profile.FilamentSlots != 4 {
		t.Fatalf("physical tool count changed: %d/%d", b.PhysicalTools, b.Profile.FilamentSlots)
	}
}

func TestFlashforgeSupportFlagNormalizationPreservesExplicitFlags(t *testing.T) {
	b := syntheticOrcaBundle(4)
	b.Slicer, b.Profile.ModelID = "Flash Studio", "Flashforge Creator 5 Pro"
	want := []any{"0", "1", "0", "0"}
	b.Profile.ProjectSettings["filament_is_support"] = want
	if err := normalizeFlashforgeCreator5ProSupportFlags(b, 1); err != nil {
		t.Fatal(err)
	}
	if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, want) {
		t.Fatalf("explicit manufacturer flags were replaced: %v", got)
	}
}

func TestFlashforgeSupportFlagNormalizationRejectsAmbiguousValues(t *testing.T) {
	for _, value := range []any{[]any{"1"}, []any{"0", "1"}, []any{}, "0"} {
		b := syntheticOrcaBundle(4)
		b.Slicer, b.Profile.ModelID = "Flash Studio", "Flashforge Creator 5 Pro"
		b.Profile.ProjectSettings["filament_is_support"] = value
		if err := normalizeFlashforgeCreator5ProSupportFlags(b, 1); err == nil {
			t.Fatalf("ambiguous support flag accepted: %v", value)
		}
		if got := b.Profile.ProjectSettings["filament_is_support"]; !reflect.DeepEqual(got, value) {
			t.Fatalf("ambiguous support flags changed: %v", got)
		}
	}
}

func TestFlashforgeSupportFlagNormalizationDoesNotTouchOtherSlicers(t *testing.T) {
	for _, tc := range []struct {
		slicer, model string
		tools         int
	}{
		{"Snapmaker Orca", "Snapmaker Artisan", 4},
		{"OrcaSlicer", "Flashforge Creator 5 Pro", 4},
		{"Flash Studio", "Flashforge AD5X", 4},
		{"Flash Studio", "Flashforge Creator 5 Pro", 1},
		{"Creality Print", "Creality K2 Plus", 1},
	} {
		b := syntheticOrcaBundle(tc.tools)
		b.Slicer, b.Profile.ModelID = tc.slicer, tc.model
		if err := normalizeFlashforgeCreator5ProSupportFlags(b, 1); err != nil {
			t.Fatal(err)
		}
		if _, exists := b.Profile.ProjectSettings["filament_is_support"]; exists {
			t.Fatalf("unrelated %s / %s received support flags", tc.slicer, tc.model)
		}
	}
}

func TestOrcaInstalledNarrowNozzle025ConvertsBambu02WithoutChangingPhysicalNozzle(t *testing.T) {
	b := syntheticOrcaBundle(1)
	b.Nozzle = "0.25"
	b.Profile.ProjectSettings["nozzle_diameter"] = []any{"0.25"}
	src := filepath.Join("..", "..", "tests", "fixtures", "bambu-nozzle-0.2.3mf")
	out := filepath.Join(t.TempDir(), "narrow-converted.3mf")
	result, err := ConvertWithOrcaBundle(src, out, writeOrcaBundle(t, b))
	if err != nil {
		t.Fatal(err)
	}
	if result["source_nozzle"] != "0.2" || result["target_nozzle"] != "0.25" || result["nozzle_remapped"] != true {
		t.Fatalf("source 0.2 -> physical 0.25 is not preserved: %v", result)
	}
	if st, err := os.Stat(out); err != nil || st.Size() == 0 {
		t.Fatalf("converted project missing: %v", err)
	}
}

func TestObservedOrcaMultitoolSupportFlagsMatchNativePalette(t *testing.T) {
	for _, tc := range []struct {
		prefix string
		tools  int
		source int
	}{
		{"orca-prusa-", 5, 1},
		{"orca-ratrig-", 2, 1},
		{"orca-volumic-", 2, 3},
		{"orca-re3d-", 2, 1},
	} {
		b := syntheticOrcaBundle(tc.tools)
		b.Target = tc.prefix + "sample-0123456789"
		b.Slicer = "OrcaSlicer"
		nozzles := append([]any(nil), b.Profile.ProjectSettings["nozzle_diameter"].([]any)...)
		if err := normalizeObservedOrcaMultitoolSupportFlags(b, tc.source); err != nil {
			t.Fatal(err)
		}
		want := make([]any, max(tc.tools, tc.source))
		for i := range want {
			want[i] = "0"
		}
		if !reflect.DeepEqual(b.Profile.ProjectSettings["filament_is_support"], want) {
			t.Fatalf("%s flags = %v, want %v", tc.prefix, b.Profile.ProjectSettings["filament_is_support"], want)
		}
		if !reflect.DeepEqual(b.Profile.ProjectSettings["nozzle_diameter"], nozzles) || b.PhysicalTools != tc.tools {
			t.Fatalf("%s changed physical nozzle or tool count", tc.prefix)
		}
	}
}

func TestObservedOrcaMultitoolFlagsPreserveExplicitOrFailClosed(t *testing.T) {
	for _, tc := range []struct {
		raw     any
		slots   int
		wantErr bool
		want    any
	}{
		{[]any{"0", "1"}, 1, false, []any{"0", "1"}},
		{[]any{"0"}, 3, false, []any{"0", "0", "0"}},
		{"0", 1, false, []any{"0", "0"}},
		{[]any{"0", "1"}, 3, true, []any{"0", "1"}},
		{[]any{"1"}, 3, true, []any{"1"}},
		{"1", 2, true, "1"},
		{[]any{}, 2, true, []any{}},
		{42, 2, true, 42},
	} {
		b := syntheticOrcaBundle(2)
		b.Slicer = "OrcaSlicer"
		b.Target = "orca-volumic-sample-0123456789"
		b.Profile.ProjectSettings["filament_is_support"] = tc.raw
		err := normalizeObservedOrcaMultitoolSupportFlags(b, tc.slots)
		if (err != nil) != tc.wantErr {
			t.Fatalf("%#v slots=%d: error %v", tc.raw, tc.slots, err)
		}
		if !reflect.DeepEqual(b.Profile.ProjectSettings["filament_is_support"], tc.want) {
			t.Fatalf("%#v slots=%d: flag result %v, want %v", tc.raw, tc.slots, b.Profile.ProjectSettings["filament_is_support"], tc.want)
		}
	}
}

func TestObservedOrcaMultitoolFlagsDoNotAlterOtherSlicersOrVendors(t *testing.T) {
	for _, tc := range []struct {
		slicer string
		target string
		tools  int
	}{
		{"Snapmaker Orca", "orca-ratrig-sample-0123456789", 2},
		{"Creality Print", "orca-volumic-sample-0123456789", 2},
		{"Flash Studio", "orca-re3d-sample-0123456789", 4},
		{"OrcaSlicer", "orca-anycubic-sample-0123456789", 2},
		{"OrcaSlicer", "orca-ratrig-sample-0123456789", 1},
	} {
		b := syntheticOrcaBundle(tc.tools)
		b.Slicer, b.Target = tc.slicer, tc.target
		if err := normalizeObservedOrcaMultitoolSupportFlags(b, 3); err != nil {
			t.Fatal(err)
		}
		if _, exists := b.Profile.ProjectSettings["filament_is_support"]; exists {
			t.Fatalf("unrelated %s/%s/%d unexpectedly received support flags", tc.slicer, tc.target, tc.tools)
		}
	}
}

func TestObservedLegacyOrcaProjectCompatibilityChangesOnlyReproducedFields(t *testing.T) {
	tests := []struct {
		name, id, model, key string
		original, expected   string
	}{
		{"Anycubic Max", "orca-anycubic-kobra-max-0123456789", "Anycubic Kobra Max",
			"layer_change_gcode", ";AFTER_LAYER_CHANGE\n;[layer_z]", ";AFTER_LAYER_CHANGE\n;[layer_z]\nG92 E0\n"},
		{"Anycubic Plus", "orca-anycubic-kobra-plus-0123456789", "Anycubic Kobra Plus",
			"layer_change_gcode", ";AFTER_LAYER_CHANGE\n;[layer_z]", ";AFTER_LAYER_CHANGE\n;[layer_z]\nG92 E0\n"},
		{"Anycubic Vyper", "orca-anycubic-vyper-0123456789", "Anycubic Vyper",
			"layer_change_gcode", ";AFTER_LAYER_CHANGE\n;[layer_z]", ";AFTER_LAYER_CHANGE\n;[layer_z]\nG92 E0\n"},
		{"Dremel 3D40", "orca-dremel-3d40-0123456789", "Dremel 3D40",
			"before_layer_change_gcode", ";BEFORE_LAYER_CHANGE\n;[layer_z]\nG92 E0\n", ";BEFORE_LAYER_CHANGE\n;[layer_z]\n"},
		{"Dremel 3D45", "orca-dremel-3d45-0123456789", "Dremel 3D45",
			"before_layer_change_gcode", ";BEFORE_LAYER_CHANGE\n;[layer_z]\nG92 E0\n", ";BEFORE_LAYER_CHANGE\n;[layer_z]\n"},
		{"FLSun QQ-S Pro", "orca-flsun-qq-s-pro-0123456789", "FLSun QQ-S Pro",
			"machine_start_gcode", "G28\nM117 Print [output_filename_format]\nM104 S140", "G28\nM117 Print started\nM104 S140"},
		{"RH3D E3NG", "orca-rh3d-e3ng-0123456789", "E3NG v1.2S",
			"wipe", "1", "0"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			b := syntheticOrcaBundle(1)
			b.Target, b.Slicer, b.Profile.ModelID = tc.id, "OrcaSlicer", tc.model
			b.Profile.ProjectSettings[tc.key] = tc.original
			b.Profile.ProjectSettings["use_firmware_retraction"] = "1"
			b.Profile.ProjectSettings["machine_end_gcode"] = "M104 S0 ; vendor end"
			nozzles := append([]any(nil), b.Profile.ProjectSettings["nozzle_diameter"].([]any)...)
			if err := normalizeObservedOrcaLegacyProjectCompatibility(b); err != nil {
				t.Fatal(err)
			}
			if got := b.Profile.ProjectSettings[tc.key]; got != tc.expected {
				t.Fatalf("%s: %s=%q, want %q", tc.name, tc.key, got, tc.expected)
			}
			if err := normalizeObservedOrcaLegacyProjectCompatibility(b); err != nil {
				t.Fatal(err)
			}
			if got := b.Profile.ProjectSettings[tc.key]; got != tc.expected {
				t.Fatalf("repeated normalization changed the project: %v", got)
			}
			if !reflect.DeepEqual(b.Profile.ProjectSettings["nozzle_diameter"], nozzles) || b.PhysicalTools != 1 ||
				b.Profile.ProjectSettings["machine_end_gcode"] != "M104 S0 ; vendor end" {
				t.Fatal("physical nozzle, tool count or manufacturer end script modified")
			}
		})
	}
}

func TestObservedLegacyOrcaUnrelatedFamiliesStayUnchanged(t *testing.T) {
	for _, tc := range []struct {
		slicer, model, target string
		tools                 int
	}{
		{"Snapmaker Orca", "Anycubic Kobra Max", "orca-anycubic-sample-0123456789", 1},
		{"Creality Print", "Dremel 3D40", "orca-dremel-sample-0123456789", 1},
		{"OrcaSlicer", "Anycubic Kobra Max", "orca-dremel-sample-0123456789", 1},
		{"OrcaSlicer", "Other Anycubic", "orca-anycubic-sample-0123456789", 1},
		{"OrcaSlicer", "FLSun QQ-S Pro", "orca-flsun-sample-0123456789", 2},
		{"OrcaSlicer", "E3NG v1.2S", "orca-rh3d-sample-0123456789", 2},
	} {
		b := syntheticOrcaBundle(tc.tools)
		b.Slicer, b.Profile.ModelID, b.Target = tc.slicer, tc.model, tc.target
		b.Profile.ProjectSettings["layer_change_gcode"] = ";AFTER_LAYER_CHANGE\n;[layer_z]"
		b.Profile.ProjectSettings["before_layer_change_gcode"] = ";BEFORE_LAYER_CHANGE\n;[layer_z]\nG92 E0\n"
		b.Profile.ProjectSettings["machine_start_gcode"] = "M117 Print [output_filename_format]"
		b.Profile.ProjectSettings["wipe"] = "1"
		b.Profile.ProjectSettings["use_firmware_retraction"] = "1"
		prior, err := json.Marshal(b.Profile.ProjectSettings)
		if err != nil {
			t.Fatal(err)
		}
		if err := normalizeObservedOrcaLegacyProjectCompatibility(b); err != nil {
			t.Fatal(err)
		}
		after, err := json.Marshal(b.Profile.ProjectSettings)
		if err != nil {
			t.Fatal(err)
		}
		if string(after) != string(prior) {
			t.Fatalf("unrelated %s / %s / %s was modified", tc.slicer, tc.model, tc.target)
		}
	}
}

func TestObservedLegacyOrcaCustomScriptsAreNotRewritten(t *testing.T) {
	for _, tc := range []struct{ model, target, key, input string }{
		{"Anycubic Kobra Plus", "orca-anycubic-plus-0123456789", "layer_change_gcode", "G28 ; custom layer script"},
		{"Dremel 3D45", "orca-dremel-3d45-0123456789", "before_layer_change_gcode", "G92 E0\nM117 custom"},
		{"FLSun QQ-S Pro", "orca-flsun-qq-s-0123456789", "machine_start_gcode", "G28\nM117 custom start"},
	} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID, b.Target = "OrcaSlicer", tc.model, tc.target
		b.Profile.ProjectSettings[tc.key] = tc.input
		if err := normalizeObservedOrcaLegacyProjectCompatibility(b); err != nil {
			t.Fatal(err)
		}
		if got := b.Profile.ProjectSettings[tc.key]; got != tc.input {
			t.Fatalf("modified unknown manufacturer script for %s: %v", tc.model, got)
		}
	}
}

func TestObservedLegacyRH3DFailsClosedOnUnknownWipe(t *testing.T) {
	for _, raw := range []any{[]any{"1"}, "maybe", true, nil} {
		b := syntheticOrcaBundle(1)
		b.Slicer, b.Profile.ModelID, b.Target = "OrcaSlicer", "E3NG v1.2S", "orca-rh3d-e3ng-0123456789"
		b.Profile.ProjectSettings["use_firmware_retraction"] = "1"
		b.Profile.ProjectSettings["wipe"] = raw
		if err := normalizeObservedOrcaLegacyProjectCompatibility(b); err == nil {
			t.Fatalf("ambiguous RH3D wipe %v accepted", raw)
		}
		if !reflect.DeepEqual(b.Profile.ProjectSettings["wipe"], raw) {
			t.Fatalf("ambiguous wipe %v changed", raw)
		}
	}
}

func TestObservedAnycubicInactiveCutNoActiveMechanicsModified(t *testing.T) {
	tests := []struct {
		model, id, nozzle string
		nullFlush         bool
	}{
		{"Anycubic Kobra S1 Max", "orca-anycubic-s1-025-0123456789", "0.25", true},
		{"Anycubic Kobra S1 Max", "orca-anycubic-s1-04-0123456789", "0.4", false},
		{"Anycubic Kobra S1 Max", "orca-anycubic-s1-06-0123456789", "0.6", true},
		{"Anycubic Kobra S1 Max", "orca-anycubic-s1-08-0123456789", "0.8", true},
		{"Anycubic Kobra X", "orca-anycubic-kobra-x-0123456789", "0.4", false},
	}
	for _, tc := range tests {
		t.Run(tc.id, func(t *testing.T) {
			b := syntheticOrcaBundle(1)
			b.Target, b.Slicer, b.Profile.ModelID, b.Nozzle = tc.id, "OrcaSlicer", tc.model, tc.nozzle
			cfg := b.Profile.ProjectSettings
			cfg["nozzle_diameter"] = []any{tc.nozzle}
			cfg["retraction_distances_when_cut"] = []any{"0"}
			cfg["long_retractions_when_cut"] = []any{"0"}
			cfg["enable_long_retraction_when_cut"] = "0"
			cfg["change_filament_gcode"] = "M400\nM600 ; installed manufacturer command"
			cfg["machine_start_gcode"] = "G28\nM104 S150"
			cfg["machine_end_gcode"] = "M104 S0\nM84"
			if tc.nullFlush {
				cfg["filament_flush_temp"] = []any{"nil"}
			} else {
				cfg["filament_flush_temp"] = []any{"245"} // Preserve explicitly valid thermal values.
			}
			if err := normalizeObservedAnycubicInactiveCutFields(b, 1); err != nil {
				t.Fatal(err)
			}
			if _, exists := cfg["retraction_distances_when_cut"]; exists {
				t.Fatal("unused zero cut length was not removed")
			}
			if cfg["long_retractions_when_cut"].([]any)[0] != "0" ||
				cfg["enable_long_retraction_when_cut"] != "0" {
				t.Fatal("active vendor cutter flags were modified")
			}
			if !reflect.DeepEqual(cfg["nozzle_diameter"], []any{tc.nozzle}) ||
				b.Nozzle != tc.nozzle || b.PhysicalTools != 1 {
				t.Fatal("physical nozzle/tool changed")
			}
			if cfg["change_filament_gcode"] != "M400\nM600 ; installed manufacturer command" ||
				cfg["machine_start_gcode"] != "G28\nM104 S150" ||
				cfg["machine_end_gcode"] != "M104 S0\nM84" {
				t.Fatal("manufacturer firmware scripts were modified")
			}
			_, foundFlush := cfg["filament_flush_temp"]
			if tc.nullFlush == foundFlush {
				t.Fatal("null flush override was not resolved exactly as expected")
			}
			if !tc.nullFlush && cfg["filament_flush_temp"].([]any)[0] != "245" {
				t.Fatal("explicit flush temperature was modified")
			}
			before, err := json.Marshal(cfg)
			if err != nil {
				t.Fatal(err)
			}
			if err := normalizeObservedAnycubicInactiveCutFields(b, 1); err != nil {
				t.Fatal(err)
			}
			after, err := json.Marshal(cfg)
			if err != nil {
				t.Fatal(err)
			}
			if string(before) != string(after) {
				t.Fatal("normalization is not idempotent")
			}
		})
	}
}

func TestObservedAnycubicCutScopeNeverTouchesActiveOrMultiMaterialProfiles(t *testing.T) {
	for _, tc := range []struct {
		slicer, target, model string
		tools, sourceSlots    int
	}{
		{"Creality Print", "orca-anycubic-model-0123456789", "Anycubic Kobra X", 1, 1},
		{"OrcaSlicer", "orca-creality-model-0123456789", "Anycubic Kobra X", 1, 1},
		{"OrcaSlicer", "orca-anycubic-model-0123456789", "Anycubic Kobra 3", 1, 1},
		{"OrcaSlicer", "orca-anycubic-model-0123456789", "Anycubic Kobra S1 Max", 2, 1},
		{"OrcaSlicer", "orca-anycubic-model-0123456789", "Anycubic Kobra S1 Max", 1, 3},
		{"OrcaSlicer", "orca-creality-model-0123456789", "Creality SPARKX i7", 1, 1},
	} {
		b := syntheticOrcaBundle(tc.tools)
		b.Target, b.Slicer, b.Profile.ModelID = tc.target, tc.slicer, tc.model
		cfg := b.Profile.ProjectSettings
		cfg["retraction_distances_when_cut"] = []any{"0"}
		cfg["long_retractions_when_cut"] = []any{"0"}
		cfg["enable_long_retraction_when_cut"] = "0"
		cfg["filament_flush_temp"] = []any{"nil"}
		before, err := json.Marshal(cfg)
		if err != nil {
			t.Fatal(err)
		}
		if err := normalizeObservedAnycubicInactiveCutFields(b, tc.sourceSlots); err != nil {
			t.Fatal(err)
		}
		after, err := json.Marshal(cfg)
		if err != nil {
			t.Fatal(err)
		}
		if string(before) != string(after) {
			t.Fatalf("changed excluded profile %s / %s / %d material(s)", tc.slicer, tc.model, tc.sourceSlots)
		}
	}
}

func TestObservedAnycubicCutFailsClosedOnUncertainMachineSettings(t *testing.T) {
	type badCase struct {
		name   string
		mutate func(map[string]any)
	}
	for _, tc := range []badCase{
		{"enabled cut flag", func(c map[string]any) { c["long_retractions_when_cut"] = []any{"1"} }},
		{"enabled global cut flag", func(c map[string]any) { c["enable_long_retraction_when_cut"] = "1" }},
		{"unknown per-extruder flags", func(c map[string]any) { c["long_retractions_when_cut"] = []any{"0", "1"} }},
		{"unknown nullable flush", func(c map[string]any) { c["filament_flush_temp"] = "nil" }},
		{"unknown multi-flush values", func(c map[string]any) { c["filament_flush_temp"] = []any{"nil", "220"} }},
		{"cut length referenced by firmware script", func(c map[string]any) {
			c["change_filament_gcode"] = "G1 E-{retraction_distances_when_cut[0]}"
		}},
		{"flush temperature referenced by firmware script", func(c map[string]any) {
			c["machine_start_gcode"] = "M109 S{filament_flush_temp[0]}"
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := syntheticOrcaBundle(1)
			b.Target, b.Slicer, b.Profile.ModelID = "orca-anycubic-kobra-s1-0123456789", "OrcaSlicer", "Anycubic Kobra S1 Max"
			cfg := b.Profile.ProjectSettings
			cfg["retraction_distances_when_cut"] = []any{"0"}
			cfg["long_retractions_when_cut"] = []any{"0"}
			cfg["enable_long_retraction_when_cut"] = "0"
			cfg["filament_flush_temp"] = []any{"nil"}
			tc.mutate(cfg)
			before, err := json.Marshal(cfg)
			if err != nil {
				t.Fatal(err)
			}
			if err := normalizeObservedAnycubicInactiveCutFields(b, 1); err == nil {
				t.Fatal("unsafe active or script-dependent mechanical settings accepted")
			}
			after, err := json.Marshal(cfg)
			if err != nil {
				t.Fatal(err)
			}
			if string(before) != string(after) {
				t.Fatal("changed machine settings on fail-closed result")
			}
		})
	}
}

func TestObservedAnycubicCutPreservesAllOtherManufacturerLengths(t *testing.T) {
	for _, original := range []any{[]any{"12"}, []any{"18"}, []any{"28"}, []any{"30"}, "0", []any{"0", "0"}} {
		b := syntheticOrcaBundle(1)
		b.Target, b.Slicer, b.Profile.ModelID = "orca-anycubic-kobra-x-0123456789", "OrcaSlicer", "Anycubic Kobra X"
		cfg := b.Profile.ProjectSettings
		cfg["retraction_distances_when_cut"] = original
		cfg["long_retractions_when_cut"] = []any{"0"}
		cfg["enable_long_retraction_when_cut"] = "0"
		if err := normalizeObservedAnycubicInactiveCutFields(b, 1); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(cfg["retraction_distances_when_cut"], original) {
			t.Fatalf("rewrote non-documented manufacturer cut distance %v", original)
		}
	}
}

func TestObservedAnycubicInactiveCutChecksVendorScriptArrays(t *testing.T) {
	for _, tc := range []struct {
		label  string
		script []any
		fail   bool
	}{
		{"safe manufacturer array", []any{"; manufacturer filament start\nM104 S190"}, false},
		{"cut value used by script array", []any{"G1 E-{retraction_distances_when_cut[0]}"}, true},
		{"flush value used by script array", []any{"M109 S{filament_flush_temp[0]}"}, true},
		{"invalid script-array value", []any{nil}, true},
	} {
		t.Run(tc.label, func(t *testing.T) {
			b := syntheticOrcaBundle(1)
			b.Target, b.Slicer, b.Profile.ModelID = "orca-anycubic-kobra-x-0123456789", "OrcaSlicer", "Anycubic Kobra X"
			cfg := b.Profile.ProjectSettings
			cfg["retraction_distances_when_cut"] = []any{"0"}
			cfg["long_retractions_when_cut"] = []any{"0"}
			cfg["enable_long_retraction_when_cut"] = "0"
			cfg["filament_start_gcode"] = tc.script
			err := normalizeObservedAnycubicInactiveCutFields(b, 1)
			if (err != nil) != tc.fail {
				t.Fatalf("script array validation: err=%v, fail=%v", err, tc.fail)
			}
			_, remains := cfg["retraction_distances_when_cut"]
			if remains != tc.fail {
				t.Fatal("script verification did not preserve or omit the inactive cut field as expected")
			}
			if !reflect.DeepEqual(cfg["filament_start_gcode"], tc.script) {
				t.Fatal("manufacturer filament script changed")
			}
		})
	}
}
