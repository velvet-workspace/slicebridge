package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

// Go remains selectable per request even if a developer set a Rust default
// elsewhere. A pilot cannot change the engine of another active conversion.
func TestRustNativeWailsRequestEngineAndRollback(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed manufacturer profiles, read only")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires approved Rust binary")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "rust-native")
	input := loadFixture(t, "bambu-one-color.3mf")
	original, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	app := &App{}
	goReply := app.Convert(ConvertRequest{FilePath: input, Targets: []string{"qidi"}, Engine: "go"})
	if !goReply.OK || len(goReply.Results) != 1 || goReply.Results[0].Report["engine"] != "bl2std" {
		t.Fatalf("explicit Wails rollback to Go failed: %+v", goReply)
	}
	rustReply := app.Convert(ConvertRequest{FilePath: input, Targets: []string{"qidi"}, Engine: "rust-native"})
	if !rustReply.OK || len(rustReply.Results) != 1 ||
		rustReply.Results[0].Report["engine"] != "rust-native-3mf" ||
		rustReply.Results[0].Report["planner"] != "rust-native" {
		t.Fatalf("explicit Wails Rust-native conversion failed: %+v", rustReply)
	}
	if goReply.Results[0].OutputPath == rustReply.Results[0].OutputPath {
		t.Fatal("Go and Rust output paths collided")
	}
	sourceAfter, err := os.ReadFile(input)
	if err != nil || !bytes.Equal(sourceAfter, original) {
		t.Fatal("Wails engine selection changed source project")
	}
	rustAgain := app.Convert(ConvertRequest{FilePath: input, Targets: []string{"qidi"}, Engine: "rust-native"})
	if !rustAgain.OK || rustAgain.Results[0].OutputPath == rustReply.Results[0].OutputPath {
		t.Fatalf("Wails Rust-native second job overwrote previous output: %+v", rustAgain)
	}
	for _, result := range []ConvertResult{goReply.Results[0], rustReply.Results[0], rustAgain.Results[0]} {
		if _, err := os.Stat(result.OutputPath); err != nil {
			t.Fatalf("expected completed conversion %s: %v", result.OutputPath, err)
		}
	}
}

func TestRustNativeWailsCancelAndDeveloperOptIn(t *testing.T) {
	app := &App{}
	ctx, done := app.beginConversion("rust-native")
	if !app.CancelConversion() {
		t.Fatal("active Rust-native request was not cancellable")
	}
	select {
	case <-ctx.Done():
	default:
		t.Fatal("cancel API did not propagate to conversion context")
	}
	done()
	if app.CancelConversion() {
		t.Fatal("finished Rust-native request remained registered")
	}
	goCtx, goDone := app.beginConversion("go")
	defer goDone()
	if app.CancelConversion() || goCtx.Err() != nil {
		t.Fatal("Go default must not be presented as cancellable")
	}
	if _, err := effectiveConversionEngine("not-an-engine"); err == nil {
		t.Fatal("invalid engine selection accepted")
	}
}

func TestRustNativeWailsAppDoesNotUseBuiltinGoPlanner(t *testing.T) {
	t.Setenv("SLICEBRIDGE_CONVERSION_ENGINE", "")
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", "")
	source := loadFixture(t, "bambu-one-color.3mf")
	response := (&App{}).Convert(ConvertRequest{
		FilePath: source, Targets: []string{"u1"}, Engine: "rust-native",
	})
	if response.OK || len(response.Results) != 0 || len(response.Failures) != 1 ||
		response.Failures[0].Code != "TARGET_PROFILE_REQUIRED" {
		t.Fatalf("opt-in Rust-native silently accepted Go builtin planner: %+v", response)
	}
}

func TestRustNativeWailsBatchMultiProjectGoParity(t *testing.T) {
	if os.Getenv("BRIDGE_TEST_INSTALLED") != "1" {
		t.Skip("requires installed manufacturer profiles, read only")
	}
	binary := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if binary == "" {
		t.Skip("requires the approved Rust-native binary")
	}
	t.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	sources := []string{
		loadFixture(t, "bambu-one-color.3mf"),
		loadFixture(t, "bambu-five-colors.3mf"),
	}
	sourceCopies := make([][]byte, len(sources))
	for i, src := range sources {
		copy, err := os.ReadFile(src)
		if err != nil {
			t.Fatal(err)
		}
		sourceCopies[i] = copy
	}
	app := &App{}
	request := BatchRequest{FilePaths: sources, Targets: []string{"u1", "qidi"}, Engine: "go"}
	baseline := app.ConvertBatch(request)
	if baseline.Error != "" || baseline.Failures != 0 || baseline.Successes != 4 ||
		baseline.FilesDone != 2 || len(baseline.Files) != 2 {
		t.Fatalf("Wails Go batch baseline refused: %+v", baseline)
	}
	request.Engine = "rust-native"
	native := app.ConvertBatch(request)
	if native.Error != "" || native.Failures != 0 || native.Successes != 4 ||
		native.FilesDone != 2 || len(native.Files) != 2 {
		t.Fatalf("Wails Rust-native batch refused: %+v", native)
	}
	for i := range sources {
		originalAfter, err := os.ReadFile(sources[i])
		if err != nil || !bytes.Equal(originalAfter, sourceCopies[i]) {
			t.Fatalf("batch modified original 3MF %s", sources[i])
		}
		if len(baseline.Files[i].Results) != 2 || len(native.Files[i].Results) != 2 {
			t.Fatalf("unexpected per-project target counts: Go=%+v Rust=%+v", baseline.Files[i], native.Files[i])
		}
		for j, rustResult := range native.Files[i].Results {
			goResult := baseline.Files[i].Results[j]
			if rustResult.Target != goResult.Target ||
				rustResult.Report["engine"] != "rust-native-3mf" ||
				rustResult.Report["planner"] != "rust-native" ||
				goResult.Report["engine"] != "bl2std" {
				t.Fatalf("batch route engine mismatch: Go=%+v Rust=%+v", goResult, rustResult)
			}
			if rustResult.OutputPath == goResult.OutputPath {
				t.Fatal("Go and Rust batch outputs overwrote one another")
			}
			got := appRead3MFParts(t, rustResult.OutputPath)
			want := appRead3MFParts(t, goResult.OutputPath)
			if len(got) != len(want) {
				t.Fatalf("%s ZIP member count changed", rustResult.Target)
			}
			for name, left := range want {
				right, ok := got[name]
				if !ok {
					t.Fatalf("%s missing member %s", rustResult.Target, name)
				}
				if bytes.Equal(left, right) {
					continue
				}
				if name == "Metadata/project_settings.config" {
					var first, second map[string]any
					if err := json.Unmarshal(left, &first); err != nil {
						t.Fatal(err)
					}
					if err := json.Unmarshal(right, &second); err != nil {
						t.Fatal(err)
					}
					if !reflect.DeepEqual(first, second) {
						t.Fatalf("%s project settings mismatch", rustResult.Target)
					}
				} else if strings.HasSuffix(name, ".xml") ||
					strings.HasSuffix(name, ".rels") ||
					strings.HasSuffix(name, ".model") ||
					name == "Metadata/model_settings.config" ||
					name == "Metadata/slice_info.config" {
					if !reflect.DeepEqual(nativeRustXMLTokens(t, left), nativeRustXMLTokens(t, right)) {
						t.Fatalf("%s XML mismatch in %s", rustResult.Target, name)
					}
				} else {
					t.Fatalf("%s binary ZIP member differs: %s", rustResult.Target, name)
				}
			}
		}
	}
}
