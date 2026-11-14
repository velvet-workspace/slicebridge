package main

import (
	"bytes"
	"os"
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
