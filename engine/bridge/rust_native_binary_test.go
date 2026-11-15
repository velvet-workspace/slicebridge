package bridge

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestRustNativePackagedSidecarUsesOnlyAdjacentRegularExecutable(t *testing.T) {
	dir := t.TempDir()
	app := filepath.Join(dir, "SliceBridge.exe")
	packed := filepath.Join(dir, rustNativeSidecarName())
	// An unrelated binary on PATH or in a different directory must not be
	// discovered as a fallback for an absent app sidecar.
	if _, err := bundledRustBinaryAt(app); err == nil {
		t.Fatal("accepted an unbundled Rust runtime")
	}
	if err := os.WriteFile(packed, []byte("test stand-in"), 0700); err != nil {
		t.Fatal(err)
	}
	got, err := bundledRustBinaryAt(app)
	if err != nil || got != packed {
		t.Fatalf("valid adjacent Rust sidecar not found: %q %v", got, err)
	}
	if err := os.Remove(packed); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(packed, 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := bundledRustBinaryAt(app); err == nil {
		t.Fatal("accepted sidecar directory as executable")
	}
	if err := os.Remove(packed); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(packed, nil, 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := bundledRustBinaryAt(app); err == nil {
		t.Fatal("accepted empty sidecar executable")
	}
}

func TestRustNativePackagedSidecarRejectsSymlinksAndWrongPermissions(t *testing.T) {
	dir := t.TempDir()
	app := filepath.Join(dir, "SliceBridge.exe")
	packed := filepath.Join(dir, rustNativeSidecarName())
	foreign := filepath.Join(dir, "foreign-binary")
	if err := os.WriteFile(foreign, []byte("test stand-in"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(foreign, packed); err == nil {
		if _, err := bundledRustBinaryAt(app); err == nil {
			t.Fatal("accepted unverified symlink for bundled sidecar")
		}
		if err := os.Remove(packed); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(packed, []byte("test stand-in"), 0600); err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" {
		if _, err := bundledRustBinaryAt(app); err == nil {
			t.Fatal("accepted nonexecutable macOS/Linux sidecar")
		}
	}
}

func TestRustNativeExplicitOverrideFailsClosedInsteadOfFallingBack(t *testing.T) {
	dir := t.TempDir()
	approved := filepath.Join(dir, "native-runtime")
	if err := os.WriteFile(approved, []byte("test stand-in"), 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv(rustNativeBinaryEnv, "")
	if _, err := resolveRustNativeBinary(); err == nil {
		t.Fatal("empty explicit override unexpectedly fell through")
	}
	t.Setenv(rustNativeBinaryEnv, "not-an-absolute-path")
	if _, err := resolveRustNativeBinary(); err == nil {
		t.Fatal("relative explicit runtime accepted")
	}
	t.Setenv(rustNativeBinaryEnv, approved)
	if got, err := resolveRustNativeBinary(); err != nil || got != approved {
		t.Fatalf("absolute explicit runtime not honored: %q %v", got, err)
	}
}
