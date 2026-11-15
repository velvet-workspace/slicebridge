package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"time"
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

// VerifyRustNativeBundle is invoked by the actual packaged Wails executable's
// headless release smoke. It verifies the exact adjacent worker that will be
// launched when a user explicitly opts into Rust-native conversion; neither
// an environment override nor PATH may spoof this package check.
func VerifyRustNativeBundle() error {
	executable, err := os.Executable()
	if err != nil {
		return fmt.Errorf("cannot locate packaged Wails executable: %w", err)
	}
	rust, err := bundledRustBinaryAt(executable)
	if err != nil {
		return err
	}
	return verifyRustNativeHandshake(rust)
}

func verifyRustNativeHandshake(binary string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, binary, "--identify-native-engine")
	command.WaitDelay = 1 * time.Second
	stdout := &cappedRustTranscript{limit: 4096}
	stderr := &cappedRustTranscript{limit: 4096}
	command.Stdout = stdout
	command.Stderr = stderr
	if err := command.Run(); err != nil {
		if ctx.Err() != nil {
			return fmt.Errorf("packaged Rust-native worker handshake timed out: %w", ctx.Err())
		}
		return fmt.Errorf("packaged Rust-native worker refused handshake: %w", err)
	}
	if stdout.truncated || stderr.truncated {
		return fmt.Errorf("packaged Rust-native worker produced oversized handshake")
	}
	var identity struct {
		OK            bool   `json:"ok"`
		Engine        string `json:"engine"`
		NativeArchive bool   `json:"native_archive"`
	}
	if err := json.Unmarshal(bytes.TrimSpace(stdout.buffer.Bytes()), &identity); err != nil {
		return fmt.Errorf("packaged Rust-native worker returned invalid identity: %w", err)
	}
	if !identity.OK || !identity.NativeArchive || identity.Engine != "slicebridge-3mf-probe" {
		return fmt.Errorf("packaged Rust-native worker did not identify a supported converter")
	}
	return nil
}
