package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/nocturney/slicebridge/engine/bridge"
	"github.com/nocturney/slicebridge/native/presets"
)

// Opt-in real installed OEM (read-only) regression for U1 physical 0.6 nozzle.
// Never invent material limits; both engines consume the exact flattened
// bundle from the installed Snapmaker Orca files on the host.
func TestRustNativeInstalledU106AppArchiveParity(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed OEM profiles")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires explicitly approved local Rust binary")
	}
	bundle, err := presets.LoadConfigured("u1", "0.6", "auto", nil)
	if err != nil {
		t.Fatalf("installed Snapmaker U1 0.6 OEM profile invalid: %v", err)
	}
	for _, typ := range []string{"PLA", "PETG"} {
		settings := bundle.MaterialSettings[typ]
		if bundle.Profile.FilamentProfiles[typ] == "" || len(settings) == 0 {
			t.Fatalf("no real 0.6mm OEM %s material leaf", typ)
		}
		for _, key := range []string{"nozzle_temperature", "filament_flow_ratio", "filament_max_volumetric_speed"} {
			value := settings[key]
			if value == nil {
				t.Fatalf("OEM %s/%s lacks critical manufacturer value", typ, key)
			}
		}
	}
	data, err := json.Marshal(bundle)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	profile := filepath.Join(dir, "installed-u1-06.json")
	if err := os.WriteFile(profile, data, 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	for _, fixtureName := range []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"} {
		t.Run(fixtureName, func(t *testing.T) {
			src := loadFixture(t, fixtureName)
			original, err := os.ReadFile(src)
			if err != nil {
				t.Fatal(err)
			}
			goOut := filepath.Join(dir, strings.TrimSuffix(fixtureName, ".3mf")+"-go.3mf")
			rustOut := filepath.Join(dir, strings.TrimSuffix(fixtureName, ".3mf")+"-rust.3mf")
			req := bridge.TargetRequest{Target: "u1", Nozzle: "0.6", ProfileFile: profile, Engine: "go"}
			goReceipt, err := bridge.ConvertTarget(src, goOut, req)
			if err != nil || goReceipt["engine"] != "bl2std" {
				t.Fatalf("Go U1/0.6 baseline invalid: %+v %v", goReceipt, err)
			}
			req.Engine = "rust-native"
			rustReceipt, err := bridge.ConvertTarget(src, rustOut, req)
			if err != nil || rustReceipt["engine"] != "rust-native-3mf" || rustReceipt["planner"] != "rust-native" {
				t.Fatalf("Rust U1/0.6 app path refused verified OEM: %+v %v", rustReceipt, err)
			}
			first, second := appRead3MFParts(t, goOut), appRead3MFParts(t, rustOut)
			if len(first) != len(second) {
				t.Fatalf("U1 0.6 3MF member counts differ: %d vs %d", len(first), len(second))
			}
			for name, expected := range first {
				actual, ok := second[name]
				if !ok {
					t.Fatalf("Rust output missing %s", name)
				}
				if bytes.Equal(expected, actual) {
					continue
				}
				if name == "Metadata/project_settings.config" {
					var a, b map[string]any
					if err := json.Unmarshal(expected, &a); err != nil {
						t.Fatal(err)
					}
					if err := json.Unmarshal(actual, &b); err != nil {
						t.Fatal(err)
					}
					if !reflect.DeepEqual(a, b) {
						t.Fatalf("U1 0.6 project settings differ")
					}
				} else if strings.HasSuffix(name, ".xml") || strings.HasSuffix(name, ".rels") ||
					strings.HasSuffix(name, ".model") || name == "Metadata/model_settings.config" || name == "Metadata/slice_info.config" {
					if !reflect.DeepEqual(nativeRustXMLTokens(t, expected), nativeRustXMLTokens(t, actual)) {
						t.Fatalf("U1 0.6 ZIP XML member differs: %s", name)
					}
				} else {
					t.Fatalf("U1 0.6 3MF non-XML member differs %s", name)
				}
			}
			after, err := os.ReadFile(src)
			if err != nil || !bytes.Equal(after, original) {
				t.Fatalf("U1 0.6 original changed: %v", err)
			}
			t.Logf("INSTALLED_U106_APP_PARITY_PASS materials=%s nozzle=%s", fixtureName, bundle.Nozzle)
		})
	}
}

func TestRustNativeWailsInstalledU106NozzleSelected(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" || os.Getenv("SLICEBRIDGE_RUST_PROBE") == "" {
		t.Skip("read-only installed U1 0.6 OEM and approved Rust binary required")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", os.Getenv("SLICEBRIDGE_RUST_PROBE"))
	t.Setenv("BRIDGE_SETTINGS_FILE", filepath.Join(t.TempDir(), "config", "settings.json"))
	app := &App{}
	_, err := app.SavePrinterSettings("u1", PrinterSettings{
		Nozzle: "0.6", Slicer: "auto", Filaments: map[string]string{},
	})
	if err != nil {
		t.Fatalf("cannot select real installed 0.6 mm U1 hardware: %v", err)
	}
	sources := []string{loadFixture(t, "bambu-one-color.3mf"), loadFixture(t, "bambu-five-colors.3mf")}
	for _, src := range sources {
		original, err := os.ReadFile(src)
		if err != nil {
			t.Fatal(err)
		}
		goBatch := app.ConvertBatch(BatchRequest{FilePaths: []string{src}, Targets: []string{"u1"}, Engine: "go"})
		rustBatch := app.ConvertBatch(BatchRequest{FilePaths: []string{src}, Targets: []string{"u1"}, Engine: "rust-native"})
		if goBatch.Error != "" || goBatch.Successes != 1 || goBatch.Failures != 0 {
			t.Fatalf("Go Wails U1 0.6 refused: %+v", goBatch)
		}
		if rustBatch.Error != "" || rustBatch.Successes != 1 || rustBatch.Failures != 0 {
			t.Fatalf("Rust Wails U1 0.6 refused: %+v", rustBatch)
		}
		first, second := goBatch.Files[0].Results[0], rustBatch.Files[0].Results[0]
		if first.Report["target_nozzle"] != "0.6" || second.Report["target_nozzle"] != "0.6" ||
			first.Report["engine"] != "bl2std" || second.Report["engine"] != "rust-native-3mf" ||
			second.Report["planner"] != "rust-native" || first.OutputPath == second.OutputPath {
			t.Fatalf("Wails converted with wrong engine or nozzle: Go=%+v Rust=%+v", first, second)
		}
		if n := len(appRead3MFParts(t, first.OutputPath)); n != len(appRead3MFParts(t, second.OutputPath)) {
			t.Fatalf("Wails Go/Rust U1 0.6 archive member counts differ")
		}
		after, err := os.ReadFile(src)
		if err != nil || !bytes.Equal(original, after) {
			t.Fatalf("Wails U1 0.6 modified source: %v", err)
		}
		t.Logf("WAILS_U106_OEM_PASS source=%s target_nozzle=0.6", filepath.Base(src))
	}
}
