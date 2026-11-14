package main

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/nocturney/slicebridge/engine/bridge"
	"github.com/nocturney/slicebridge/native/presets"
)

// Installed Flash Studio OEM only; no profile files are written back to the
// real slicer, no slicer executable is launched, and no G-code is generated.
func TestRustNativeInstalledFlashforgeOrcaWailsBatchParity(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires real installed read-only Flash Studio OEM profiles")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires approved Rust-native executable")
	}
	const id = "orca-flashforge-flashforge-adventurer-5m-0-4-30612b8651"
	bundle, err := presets.ResolveOrcaBundle(id)
	if err != nil {
		t.Skipf("verified Flash Studio Adventurer 5M preset not installed: %v", err)
	}
	if bundle.Slicer != "Flash Studio Desktop" ||
		bundle.Profile.ModelID != "Flashforge Adventurer 5M" ||
		bundle.Nozzle != "0.4" {
		t.Skipf("local installed OEM profile differs from tested identity: %+v", bundle)
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	sources := []string{
		loadFixture(t, "bambu-one-color.3mf"),
		loadFixture(t, "bambu-five-colors.3mf"),
	}
	before := make([][]byte, len(sources))
	for i, source := range sources {
		before[i], err = os.ReadFile(source)
		if err != nil {
			t.Fatal(err)
		}
	}
	app := &App{}
	goResult := app.ConvertBatch(BatchRequest{FilePaths: sources, Targets: []string{id}, Engine: "go"})
	if goResult.Error != "" || goResult.Successes != len(sources) || goResult.Failures != 0 {
		t.Fatalf("installed Orca Go baseline rejected: %+v", goResult)
	}
	rustResult := app.ConvertBatch(BatchRequest{FilePaths: sources, Targets: []string{id}, Engine: "rust-native"})
	if rustResult.Error != "" || rustResult.Successes != len(sources) || rustResult.Failures != 0 {
		t.Fatalf("installed Orca Rust-native app conversion rejected: %+v", rustResult)
	}
	for index, source := range sources {
		after, err := os.ReadFile(source)
		if err != nil || !bytes.Equal(after, before[index]) {
			t.Fatalf("source was changed: %s", source)
		}
		if len(rustResult.Files[index].Results) != 1 || len(goResult.Files[index].Results) != 1 {
			t.Fatalf("missing target result for %s", source)
		}
		goPart := goResult.Files[index].Results[0]
		rustPart := rustResult.Files[index].Results[0]
		if goPart.Report["engine"] != "bl2std" || rustPart.Report["engine"] != "rust-native-3mf" ||
			rustPart.Report["planner"] != "rust-native" || goPart.OutputPath == rustPart.OutputPath {
			t.Fatalf("unexpected app engine selection: Go=%+v Rust=%+v", goPart, rustPart)
		}
		want := appRead3MFParts(t, goPart.OutputPath)
		got := appRead3MFParts(t, rustPart.OutputPath)
		if len(want) != len(got) {
			t.Fatalf("changed archive member count: Go=%d Rust=%d", len(want), len(got))
		}
		for name, expected := range want {
			actual, found := got[name]
			if !found {
				t.Fatalf("Rust-native missing 3MF member %s", name)
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
					t.Fatalf("Rust/Go material settings differ for %s", name)
				}
			} else if strings.HasSuffix(name, ".xml") ||
				strings.HasSuffix(name, ".rels") || strings.HasSuffix(name, ".model") ||
				name == "Metadata/model_settings.config" || name == "Metadata/slice_info.config" {
				if !reflect.DeepEqual(nativeRustXMLTokens(t, expected), nativeRustXMLTokens(t, actual)) {
					t.Fatalf("Rust/Go project metadata mismatch for %s", name)
				}
			} else {
				t.Fatalf("Rust/Go non-XML ZIP member mismatch for %s", name)
			}
		}
		t.Logf("FLASHFORGE_ORCA_RUST_NATIVE_APP_PASS printer=%s source=%s", bundle.Profile.ModelID, filepath.Base(source))
	}
}

// Prevent changing slicer ownership or enabling an unported proprietary fork
// just because the opaque ID has the Flashforge prefix.
func TestRustNativeFlashforgeOrcaOtherSlicerRefused(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires a verified local installed Flash Studio profile")
	}
	const id = "orca-flashforge-flashforge-adventurer-5m-0-4-30612b8651"
	bundle, err := presets.ResolveOrcaBundle(id)
	if err != nil {
		t.Skipf("manufacturer OEM not installed: %v", err)
	}
	bundle.Slicer = "OrcaSlicer"
	data, err := json.Marshal(bundle)
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	file := filepath.Join(dir, "altered-ownership.json")
	if err := os.WriteFile(file, data, 0600); err != nil {
		t.Fatal(err)
	}
	source := loadFixture(t, "bambu-one-color.3mf")
	result, err := bridge.ConvertTargetContext(context.Background(), source, filepath.Join(dir, "refused.3mf"), bridge.TargetRequest{
		Target: id, ProfileFile: file, Engine: "rust-native",
	})
	if err == nil || bridge.ErrorCode(err) != "RUST_NATIVE_UNSUPPORTED" || result != nil {
		t.Fatalf("unverified fork ownership accepted: result=%+v err=%v", result, err)
	}
}

func TestRustNativeInstalledFlashStudioOrcaFamilyMatrix(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" || os.Getenv("SLICEBRIDGE_RUST_PROBE") == "" {
		t.Skip("local installed OEM read-only Rust-native matrix only")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", os.Getenv("SLICEBRIDGE_RUST_PROBE"))
	// IDs are hashed from *installed* OEM profiles. Missing profiles skip;
	// the app re-resolves all of them from disk before accepting conversion.
	ids := []string{
		"orca-flashforge-flashforge-ad5x-0-4-nozzle-24fc9cf515",
		"orca-flashforge-flashforge-adventurer-3-serie-f7c9993899",
		"orca-flashforge-flashforge-adventurer-5m-0-4-30612b8651",
		"orca-flashforge-flashforge-adventurer-5m-pro-d754869e9c",
		"orca-flashforge-flashforge-adventurer-a5-0-4-32f08bd505",
		"orca-flashforge-flashforge-guider-2s-0-4-nozz-6e7ff1a488",
		"orca-flashforge-flashforge-guider-3-ultra-0-4-814d9dca9e",
		"orca-flashforge-flashforge-guider4-0-4-hf-noz-925b94616c",
		"orca-flashforge-flashforge-guider4-0-4-nozzle-f9128e0a23",
		"orca-flashforge-flashforge-guider4-pro-0-4-hf-62925dc35e",
		"orca-flashforge-flashforge-guider4-pro-0-4-no-8ceda23689",
	}
	for _, id := range ids {
		t.Run(id, func(t *testing.T) {
			bundle, err := presets.ResolveOrcaBundle(id)
			if err != nil {
				t.Skipf("verified vendor OEM not installed: %v", err)
			}
			if bundle.Slicer != "Flash Studio Desktop" || bundle.Nozzle != "0.4" {
				t.Fatalf("verified OEM identity changed unexpectedly: %+v", bundle)
			}
			source := loadFixture(t, "bambu-one-color.3mf")
			app := &App{}
			baseline := app.ConvertBatch(BatchRequest{FilePaths: []string{source}, Targets: []string{id}, Engine: "go"})
			if baseline.Successes != 1 || baseline.Failures != 0 {
				t.Skipf("Go baseline rejects incomplete installed OEM leaf (not a Rust pass): %+v", baseline)
			}
			native := app.ConvertBatch(BatchRequest{FilePaths: []string{source}, Targets: []string{id}, Engine: "rust-native"})
			if native.Successes != 1 || native.Failures != 0 {
				t.Fatalf("Go accepted model but Rust-native refused or corrupted: %+v", native)
			}
			goOutput := baseline.Files[0].Results[0].OutputPath
			rustOutput := native.Files[0].Results[0].OutputPath
			if goOutput == rustOutput || native.Files[0].Results[0].Report["engine"] != "rust-native-3mf" {
				t.Fatalf("wrong engine or overwrite in verified OEM %s", id)
			}
			expected := appRead3MFParts(t, goOutput)
			actual := appRead3MFParts(t, rustOutput)
			if len(expected) != len(actual) {
				t.Fatalf("%s ZIP member count differs: Go=%d Rust=%d", id, len(expected), len(actual))
			}
			for name, a := range expected {
				b, found := actual[name]
				if !found {
					t.Fatalf("%s missing ZIP member %s", id, name)
				}
				if bytes.Equal(a, b) {
					continue
				}
				if name == "Metadata/project_settings.config" {
					var x, y map[string]any
					if err := json.Unmarshal(a, &x); err != nil {
						t.Fatal(err)
					}
					if err := json.Unmarshal(b, &y); err != nil {
						t.Fatal(err)
					}
					if !reflect.DeepEqual(x, y) {
						t.Fatalf("%s OEM settings differ in %s", id, name)
					}
				} else if strings.HasSuffix(name, ".xml") || strings.HasSuffix(name, ".rels") ||
					strings.HasSuffix(name, ".model") ||
					name == "Metadata/model_settings.config" || name == "Metadata/slice_info.config" {
					if !reflect.DeepEqual(nativeRustXMLTokens(t, a), nativeRustXMLTokens(t, b)) {
						t.Fatalf("%s XML differs in %s", id, name)
					}
				} else {
					t.Fatalf("%s non-XML ZIP member differs: %s", id, name)
				}
			}
			t.Logf("ORCA_FLASH_STUDIO_PARITY_PASS model=%s slicer=%s nozzle=%s",
				bundle.Profile.ModelID, bundle.Slicer, bundle.Nozzle)
		})
	}
}
