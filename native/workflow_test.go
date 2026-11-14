package main

import (
	"archive/zip"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/nocturney/slicebridge/native/presets"
)

func withFreshSettings(t *testing.T) {
	t.Helper()
	t.Setenv("BRIDGE_SETTINGS_FILE", filepath.Join(t.TempDir(), "configuration", "settings.json"))
}
func TestSavedPrinterSettingsRoundTrip(t *testing.T) {
	withFreshSettings(t)
	app := &App{}
	value, err := app.GetSettings()
	if err != nil {
		t.Fatal(err)
	}
	if value.Printers["cc2"].Nozzle != "auto" {
		t.Fatalf("defaults not automatic: %+v", value)
	}
	custom := PrinterSettings{Nozzle: "0.6", Slicer: "auto", Filaments: map[string]string{}}
	saved, err := app.SavePrinterSettings("cc2", custom)
	if err != nil {
		t.Fatal(err)
	}
	if saved.Printers["cc2"].Nozzle != "0.6" {
		t.Fatalf("not persisted: %+v", saved)
	}
	onDisk, err := loadSettings()
	if err != nil {
		t.Fatal(err)
	}
	if onDisk.Printers["cc2"].Nozzle != "0.6" {
		t.Fatalf("not on disk: %+v", onDisk)
	}
	if _, err = app.SavePrinterSettings("c5", PrinterSettings{Nozzle: "0.2", Slicer: "auto"}); err == nil {
		t.Fatal("unsupported physical nozzle accepted")
	}
	if _, err = app.SavePrinterSettings("cc2", PrinterSettings{Nozzle: "0.6", Slicer: "other-executable"}); err == nil {
		t.Fatal("untrusted slicer selection accepted")
	}
	after, err := loadSettings()
	if err != nil {
		t.Fatal(err)
	}
	if after.Printers["cc2"].Nozzle != "0.6" {
		t.Fatal("invalid update damaged saved settings")
	}
}
func TestSettingsWindowSizePreservedWhenPrinterSaved(t *testing.T) {
	withFreshSettings(t)
	cfg := defaultSettings()
	cfg.Window = WindowSettings{Width: 1430, Height: 890}
	if err := writeSettings(cfg); err != nil {
		t.Fatal(err)
	}
	app := &App{}
	if _, err := app.SavePrinterSettings("u1", PrinterSettings{Nozzle: "auto", Slicer: "auto"}); err != nil {
		t.Fatal(err)
	}
	got, err := loadSettings()
	if err != nil {
		t.Fatal(err)
	}
	if got.Window != cfg.Window {
		t.Fatalf("window bounds lost %+v", got.Window)
	}
}
func TestFolderQueueAndGeneratedOutputs(t *testing.T) {
	root := t.TempDir()
	nested := filepath.Join(root, "subfolder")
	if err := os.MkdirAll(nested, 0700); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join("..", "tests", "fixtures", "bambu-one-color.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"alpha.3mf", "subfolder/beta.3mf", "subfolder/U1-beta.3mf"} {
		target := filepath.Join(root, filepath.FromSlash(name))
		if err := os.WriteFile(target, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	list, err := collectInputs([]string{root})
	if err != nil {
		t.Fatal(err)
	}
	if len(list) != 2 {
		t.Fatalf("folder expansion expected 2 unconverted files, got %+v", list)
	}
	once, err := collectInputs([]string{list[0].Path, list[0].Path})
	if err != nil {
		t.Fatal(err)
	}
	if len(once) != 1 {
		t.Fatalf("duplicates were not removed: %+v", once)
	}
	manual, err := collectInputs([]string{filepath.Join(nested, "U1-beta.3mf")})
	if err != nil || len(manual) != 1 {
		t.Fatalf("manually-selected previous output should remain allowed: %v %+v", err, manual)
	}
}
func TestActualNozzlePreferenceControlsOutput(t *testing.T) {
	withFreshSettings(t)
	source := loadFixture(t, "bambu-nozzle-0.6.3mf")
	a := &App{}
	pref := PrinterSettings{Nozzle: "0.4", Slicer: "auto", Filaments: map[string]string{}}
	if _, err := a.SavePrinterSettings("u1", pref); err != nil {
		t.Fatal(err)
	}
	report := a.ConvertBatch(BatchRequest{FilePaths: []string{source}, Targets: []string{"u1"}})
	if report.Successes != 1 || report.Failures != 0 {
		t.Fatalf("batch failed %+v", report)
	}
	r := report.Files[0].Results[0]
	if r.Report["source_nozzle"] != "0.6" || r.Report["target_nozzle"] != "0.4" || r.Report["nozzle_selected_by_user"] != true {
		t.Fatalf("physical nozzle preference ignored: %+v", r.Report)
	}
	if _, err := os.Stat(r.OutputPath); err != nil {
		t.Fatal(err)
	}
	if len(a.generated) != 1 {
		t.Fatal("generated output is not registered for safe opening")
	}
	if err := a.OpenInSlicer("cc2", r.OutputPath); err == nil {
		t.Fatal("cross-printer opening was accepted")
	}
	if err := a.OpenInSlicer("u1", source); err == nil {
		t.Fatal("arbitrary source project opening was accepted")
	}
}
func TestBatchContinuesAcrossSourceFailures(t *testing.T) {
	withFreshSettings(t)
	valid := loadFixture(t, "bambu-one-color.3mf")
	invalid := filepath.Join(t.TempDir(), "not-a-project.3mf")
	if err := os.WriteFile(invalid, []byte("no 3mf model"), 0600); err != nil {
		t.Fatal(err)
	}
	a := &App{}
	result := a.ConvertBatch(BatchRequest{FilePaths: []string{invalid, valid}, Targets: []string{"u1"}})
	if result.Successes != 1 || result.Failures != 1 || result.FilesDone != 2 {
		t.Fatalf("batch should continue and report partial success: %+v", result)
	}
	if len(result.Files[0].Failures) != 1 || len(result.Files[1].Results) != 1 {
		t.Fatalf("per-file report incomplete: %+v", result)
	}
	if result.Files[1].Source.Objects < 1 {
		t.Fatalf("source object count not inspected: %+v", result.Files[1].Source)
	}
}
func TestInstalledPreferencesAndFourWayBatch(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed target slicers")
	}
	withFreshSettings(t)
	for _, target := range []string{"u1", "c5", "cc2", "qidi"} {
		names, err := presets.FilamentPresets(target, "0.4", "auto", "PLA")
		if err != nil {
			t.Errorf("%s list presets: %v", target, err)
			continue
		}
		if len(names) == 0 {
			t.Errorf("%s missing real PLA filament options", target)
			continue
		}
		if _, err := (&App{}).SavePrinterSettings(target, PrinterSettings{Nozzle: "0.4", Slicer: "auto", Filaments: map[string]string{"PLA": names[0]}}); err != nil {
			t.Errorf("%s save native preset %q: %v", target, names[0], err)
		}
	}
	paths := []string{loadFixture(t, "bambu-one-color.3mf"), loadFixture(t, "bambu-nozzle-0.2.3mf")}
	result := (&App{}).ConvertBatch(BatchRequest{FilePaths: paths, Targets: []string{"u1", "c5", "cc2", "qidi"}})
	if result.Successes != 8 || result.Failures != 0 {
		t.Fatalf("batch with saved presets failed: %+v", result)
	}
	for _, f := range result.Files {
		for _, r := range f.Results {
			if r.Report["target_nozzle"] != "0.4" {
				t.Errorf("%s output got nozzle %v not saved 0.4", r.Target, r.Report["target_nozzle"])
			}
		}
	}
}

// On a host with the vendor slicers installed, validate that even targets
// with four physical toolheads preserve all five virtual palette entries.
// No slicer UI, printer, or personal settings are changed by this test.
func TestFiveVirtualFilamentsAcrossInstalledFourTargets(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed target slicer presets")
	}
	withFreshSettings(t)
	source := loadFixture(t, "bambu-five-colors.3mf")
	report := (&App{}).ConvertBatch(BatchRequest{
		FilePaths: []string{source},
		Targets:   []string{"u1", "c5", "cc2", "qidi"},
	})
	if report.Successes != 4 || report.Failures != 0 || len(report.Files) != 1 {
		t.Fatalf("five virtual colors did not convert to all targets: %+v", report)
	}
	wantColors := []string{"#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FFFFFF"}
	for _, result := range report.Files[0].Results {
		// Virtual source filament entries are unrelated to the number of
		// physical print nozzles. The verified X-Plus 5 has one nozzle;
		// optional feeder hardware is a separate per-user capability.
		wantPhysicalTools := 4
		if result.Target == "qidi" {
			wantPhysicalTools = 1
		}
		if result.Report["virtual_filament_slots"] != 5 ||
			result.Report["physical_toolheads"] != wantPhysicalTools ||
			result.Report["user_slot_mapping_required"] != true {
			t.Errorf("%s lost correct virtual/physical tool metadata (want %d physical nozzles): %+v", result.Target, wantPhysicalTools, result.Report)
			continue
		}
		mapped, ok := result.Report["source_to_target_slot_map"].(map[int]int)
		if !ok || mapped[5] != 5 || mapped[1] != 1 {
			t.Errorf("%s silently folded fifth slot: %+v", result.Target, result.Report)
		}
		inspection, err := converter.Inspect(result.OutputPath)
		if err != nil || len(inspection.Filaments) != 5 {
			t.Errorf("%s output lost virtual filaments: %+v (%v)", result.Target, inspection, err)
			continue
		}
		for i, filament := range inspection.Filaments {
			if filament.Color != wantColors[i] || filament.Type != "PLA" {
				t.Errorf("%s slot %d changed identity: %+v", result.Target, i+1, filament)
			}
		}
		archive, err := zip.OpenReader(result.OutputPath)
		if err != nil {
			t.Fatal(err)
		}
		entries := map[string][]byte{}
		for _, f := range archive.File {
			if f.Name != "Metadata/project_settings.config" &&
				f.Name != "Metadata/model_settings.config" &&
				f.Name != "3D/3dmodel.model" &&
				f.Name != "Metadata/plate_1.gcode" {
				continue
			}
			input, err := f.Open()
			if err != nil {
				t.Fatal(err)
			}
			data, err := io.ReadAll(input)
			input.Close()
			if err != nil {
				t.Fatal(err)
			}
			entries[f.Name] = data
		}
		archive.Close()
		cfg := map[string]any{}
		if err := json.Unmarshal(entries["Metadata/project_settings.config"], &cfg); err != nil {
			t.Fatal(err)
		}
		colors, ok := cfg["filament_colour"].([]any)
		if !ok || len(colors) != 5 {
			t.Errorf("%s palette shortened: %+v", result.Target, cfg["filament_colour"])
		} else {
			for i, color := range colors {
				if color != wantColors[i] {
					t.Errorf("%s target color %d changed: %v", result.Target, i+1, color)
				}
			}
		}
		if !strings.Contains(string(entries["3D/3dmodel.model"]), "paint_color=\"2C\"") {
			t.Errorf("%s lost fifth-filament per-triangle painting", result.Target)
		}
		if !strings.Contains(string(entries["Metadata/model_settings.config"]), "value=\"5\"") {
			t.Errorf("%s lost fifth-filament object assignment", result.Target)
		}
		if _, leaked := entries["Metadata/plate_1.gcode"]; leaked {
			t.Errorf("%s copied stale printer G-code", result.Target)
		}
	}
}

func TestInvalidSettingsRecoveryBacksUpOriginalOnlyWithConsent(t *testing.T) {
	withFreshSettings(t)
	path, err := settingsPath()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	original := []byte("{ definitely not JSON }")
	if err := os.WriteFile(path, original, 0600); err != nil {
		t.Fatal(err)
	}
	app := &App{}
	if _, err := app.GetSettings(); err == nil {
		t.Fatal("invalid settings should fail closed")
	}
	if _, err := app.RecoverInvalidSettings(false); err == nil {
		t.Fatal("recovery without user confirmation was accepted")
	}
	if unchanged, err := os.ReadFile(path); err != nil || string(unchanged) != string(original) {
		t.Fatalf("declined recovery modified settings: %s, %v", unchanged, err)
	}
	recovered, err := app.RecoverInvalidSettings(true)
	if err != nil {
		t.Fatal(err)
	}
	if recovered.BackupPath == "" {
		t.Fatal("original file backup path not returned")
	}
	backupBytes, err := os.ReadFile(recovered.BackupPath)
	if err != nil || string(backupBytes) != string(original) {
		t.Fatalf("invalid settings not preserved: %s, %v", backupBytes, err)
	}
	if recovered.Settings.Version != 1 || recovered.Settings.Printers["u1"].Nozzle != "auto" {
		t.Fatalf("unexpected recovery settings: %+v", recovered.Settings)
	}
	if _, err := app.SavePrinterSettings("u1", PrinterSettings{Nozzle: "0.4", Slicer: "auto"}); err != nil {
		t.Fatalf("settings unusable after recovery: %v", err)
	}
	valid, err := app.RecoverInvalidSettings(true)
	if err != nil || valid.BackupPath != "" {
		t.Fatalf("valid settings must not be reset or backed up: %+v %v", valid, err)
	}
}

func TestFutureSettingsSchemaRecoveryPreservesOriginal(t *testing.T) {
	withFreshSettings(t)
	blob := []byte("{\"version\":99,\"printers\":{}}")
	path, err := settingsPath()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, blob, 0600); err != nil {
		t.Fatal(err)
	}
	app := &App{}
	if _, err := app.GetSettings(); err == nil {
		t.Fatal("unsupported future schema should not be silently reset")
	}
	res, err := app.RecoverInvalidSettings(true)
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(res.BackupPath)
	if err != nil || string(got) != string(blob) || res.Settings.Version != 1 {
		t.Fatalf("future-schema backup lost: %+v %s %v", res, got, err)
	}
}

func TestAutoNozzleRejectsSavedFilamentPreset(t *testing.T) {
	withFreshSettings(t)
	app := &App{}
	_, err := app.SavePrinterSettings("u1", PrinterSettings{
		Nozzle: "auto", Slicer: "auto", Filaments: map[string]string{"PLA": "PLA @U1 0.4 nozzle"},
	})
	if err == nil || !strings.Contains(err.Error(), "fixed physical nozzle") {
		t.Fatalf("ambiguous nozzle/preset combination accepted: %v", err)
	}
	got, err := app.GetSettings()
	if err != nil || len(got.Printers["u1"].Filaments) != 0 {
		t.Fatalf("invalid preset was persisted: %+v %v", got, err)
	}
}

func TestFolderImportKeepsGenuineModelNamedSourcesAndSkipsProvableOutputs(t *testing.T) {
	dir := t.TempDir()
	data, err := os.ReadFile(filepath.Join("..", "tests", "fixtures", "bambu-one-color.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	genuine := []string{
		"source.3mf", "H2D-benchy.3mf", "P1S-bracket.3mf",
		"Neptune4Pro-test.3mf", "U1-original-name.3mf",
	}
	generated := []string{
		"Creator5Pro-0.4-SB0123456789-source.3mf",
		"Creator5Pro-0.4-SB0123456789-source-2.3mf",
		"Creator5Pro-0.6-SBfedcba9876-source.3mf",
		"U1-source.3mf", "ORCA-Acme-foo-0-4-source.3mf", "PRUSA-orphan.3mf",
	}
	for _, name := range append(append([]string{}, genuine...), generated...) {
		if err := os.WriteFile(filepath.Join(dir, name), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	collected, err := collectInputs([]string{dir})
	if err != nil {
		t.Fatal(err)
	}
	found := map[string]bool{}
	for _, file := range collected {
		found[file.Name] = true
	}
	if len(found) != len(genuine) {
		t.Fatalf("wrong number of imported sources: got %v want %v", found, genuine)
	}
	for _, name := range genuine {
		if !found[name] {
			t.Errorf("genuine model-prefixed user source was silently dropped: %q", name)
		}
	}
	for _, name := range generated {
		if found[name] || !isGeneratedSiblingOutput(filepath.Join(dir, name)) {
			t.Errorf("unambiguous previously generated output was re-imported: %q", name)
		}
	}
	// Without the original source sibling the filename alone cannot prove
	// it is an output. Never discard an arbitrary user project by prefix.
	standalone := filepath.Join(t.TempDir(), "H2D-0.4-SB0123456789-orphan.3mf")
	if err := os.WriteFile(standalone, data, 0600); err != nil {
		t.Fatal(err)
	}
	if isGeneratedSiblingOutput(standalone) {
		t.Fatal("orphan may be user-authored and must not disappear silently")
	}
	reselected, err := collectInputs([]string{filepath.Join(dir, generated[0])})
	if err != nil || len(reselected) != 1 {
		t.Fatalf("manual project reselection was incorrectly blocked: %v %+v", err, reselected)
	}
}
