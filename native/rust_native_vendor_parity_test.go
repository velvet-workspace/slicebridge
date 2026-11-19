package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

// Tests real, read-only installed slicer OEM presets via the App.Convert
// backend. It neither launches a slicer nor asks one to slice or emit G-code.
func TestRustNativeInstalledC5CC2AppParity(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed read-only OEM presets")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires explicitly approved local Rust binary")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	for _, target := range []string{"c5", "cc2"} {
		for _, fixtureName := range []string{"bambu-one-color.3mf", "bambu-five-colors.3mf"} {
			t.Run(target+"_"+fixtureName, func(t *testing.T) {
				source := loadFixture(t, fixtureName)
				original, err := os.ReadFile(source)
				if err != nil {
					t.Fatal(err)
				}
				app := &App{}
				baseline := app.Convert(ConvertRequest{
					FilePath: source, Targets: []string{target}, Engine: "go",
				})
				if !baseline.OK || len(baseline.Results) != 1 ||
					baseline.Results[0].Report["engine"] != "bl2std" {
					t.Fatalf("Go installed OEM baseline refused: %+v", baseline)
				}
				native := app.Convert(ConvertRequest{
					FilePath: source, Targets: []string{target}, Engine: "rust-native",
				})
				if !native.OK || len(native.Results) != 1 ||
					native.Results[0].Report["engine"] != "rust-native-3mf" {
					t.Fatalf("Rust-native app installed OEM conversion refused: %+v", native)
				}
				goParts := appRead3MFParts(t, baseline.Results[0].OutputPath)
				rustParts := appRead3MFParts(t, native.Results[0].OutputPath)
				if len(goParts) != len(rustParts) {
					t.Fatalf("ZIP member count changed: Go=%d Rust=%d", len(goParts), len(rustParts))
				}
				for name, left := range goParts {
					right, ok := rustParts[name]
					if !ok {
						t.Fatalf("Rust output missing 3MF member %s", name)
					}
					if bytes.Equal(left, right) {
						continue
					}
					if name == "Metadata/project_settings.config" {
						var a, b map[string]any
						if err := json.Unmarshal(left, &a); err != nil {
							t.Fatal(err)
						}
						if err := json.Unmarshal(right, &b); err != nil {
							t.Fatal(err)
						}
						if !reflect.DeepEqual(a, b) {
							t.Fatalf("Rust and Go OEM project settings differ for %s", name)
						}
					} else if strings.HasSuffix(name, ".xml") ||
						strings.HasSuffix(name, ".rels") ||
						strings.HasSuffix(name, ".model") ||
						name == "Metadata/model_settings.config" ||
						name == "Metadata/slice_info.config" {
						if !reflect.DeepEqual(nativeRustXMLTokens(t, left), nativeRustXMLTokens(t, right)) {
							t.Fatalf("Rust and Go 3MF XML differs for %s", name)
						}
					} else {
						t.Fatalf("non-XML 3MF member differs: %s", name)
					}
				}
				after, err := os.ReadFile(source)
				if err != nil || !bytes.Equal(after, original) {
					t.Fatal("Rust-native app changed original source")
				}
				t.Logf("APP_OEM_NATIVE_PARITY target=%s fixture=%s", target, fixtureName)
			})
		}
	}
}
