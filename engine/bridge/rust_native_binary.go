package bridge

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
)

const rustNativeBinaryEnv = "SLICEBRIDGE_RUST_ENGINE_BIN"

func rustNativeSidecarName() string {
	if runtime.GOOS == "windows" {
		return "slicebridge-3mf-probe.exe"
	}
	return "slicebridge-3mf-probe"
}

// bundledRustBinaryAt resolves ONLY the current app's adjacent RC sidecar,
// never a random executable from PATH, the working directory, or a writable
// user-supplied location. The opt-in Rust engine remains disabled by default.
func bundledRustBinaryAt(applicationExecutable string) (string, error) {
	if !filepath.IsAbs(applicationExecutable) {
		return "", fmt.Errorf("application executable location is not absolute")
	}
	candidate := filepath.Join(filepath.Dir(applicationExecutable), rustNativeSidecarName())
	info, err := os.Lstat(candidate)
	if err != nil || !info.Mode().IsRegular() || info.Size() == 0 {
		return "", fmt.Errorf("Rust-native app sidecar is absent or invalid: %s", candidate)
	}
	if runtime.GOOS != "windows" && info.Mode()&0111 == 0 {
		return "", fmt.Errorf("Rust-native app sidecar is not executable: %s", candidate)
	}
	return candidate, nil
}

// An explicitly set variable always takes precedence, including an empty or
// invalid value. It must fail closed instead of silently executing another
// binary. With no override, the developer-opted native engine looks only
// alongside the installed/packed Wails executable.
func resolveRustNativeBinary() (string, error) {
	if explicit, set := os.LookupEnv(rustNativeBinaryEnv); set {
		if !filepath.IsAbs(explicit) {
			return "", fmt.Errorf("Rust-native requires an absolute %s", rustNativeBinaryEnv)
		}
		info, err := os.Stat(explicit)
		if err != nil || !info.Mode().IsRegular() || info.Size() == 0 {
			return "", fmt.Errorf("Rust-native explicit executable is unavailable: %s", explicit)
		}
		return explicit, nil
	}
	executable, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("cannot identify current app executable: %w", err)
	}
	return bundledRustBinaryAt(executable)
}
