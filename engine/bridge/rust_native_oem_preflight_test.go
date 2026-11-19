package bridge

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// This opt-in test consumes only explicitly supplied, already flattened,
// local OEM snapshots. It never discovers/changes user presets, slices,
// prints or treats a QIDI physical feeder as a generic material limit.
func TestRustInstalledOEMPreflightFiveVirtualMaterials(t *testing.T) {
	executable := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if executable == "" || root == "" {
		t.Skip("requires approved installed OEM snapshots and experimental Rust executable")
	}
	for _, tc := range []struct{ target, model string }{
		{"u1", "Snapmaker U1"},
		{"qidi", "X-Plus 5"},
	} {
		t.Run(tc.target, func(t *testing.T) {
			source := filepath.Join(root, fmt.Sprintf("scratch-%s-0.4-resolved.json", tc.target))
			raw, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			before := sha256.Sum256(raw)
			types := []string{"PLA", "PLA", "PLA", "PLA", "PETG"}
			filaments := make([]map[string]any, 0, len(types))
			for i, material := range types {
				filaments = append(filaments, map[string]any{"id": i + 1, "type": material})
			}
			invoke := func(mutator func(map[string]any)) (map[string]any, error) {
				t.Helper()
				var bundle map[string]any
				if err := json.Unmarshal(raw, &bundle); err != nil {
					t.Fatal(err)
				}
				if mutator != nil {
					mutator(bundle)
				}
				payload, err := json.Marshal(map[string]any{"bundle": bundle, "filaments": filaments})
				if err != nil {
					t.Fatal(err)
				}
				path := filepath.Join(t.TempDir(), "oem-preflight.json")
				if err := os.WriteFile(path, payload, 0600); err != nil {
					t.Fatal(err)
				}
				ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
				defer cancel()
				out, runErr := exec.CommandContext(ctx, executable, "--validate-native-bundle", path).Output()
				if ctx.Err() != nil {
					t.Fatal(ctx.Err())
				}
				var reply map[string]any
				if err := json.Unmarshal(out, &reply); err != nil {
					t.Fatalf("invalid Rust JSON receipt: %v / %.250s", err, out)
				}
				return reply, runErr
			}
			good, err := invoke(nil)
			if err != nil || good["ok"] != true || good["experimental"] != true {
				t.Fatalf("valid OEM bundle rejected: err=%v response=%v", err, good)
			}
			validation, ok := good["validation"].(map[string]any)
			if !ok {
				t.Fatalf("missing validation receipt: %v", good)
			}
			machine, ok := validation["machine"].(map[string]any)
			if !ok || machine["model_id"] != tc.model || machine["target"] != tc.target || machine["nozzle_diameter"] != 0.4 {
				t.Fatalf("installed OEM identity changed: %v", machine)
			}
			if validation["virtual_filament_slots"] != float64(5) {
				t.Fatalf("five source materials were not kept separate: %v", validation)
			}
			if _, leaks := validation["project_settings"]; leaks {
				t.Fatal("Rust preflight leaked private machine settings")
			}
			bad, err := invoke(func(bundle map[string]any) {
				settings := bundle["material_settings"].(map[string]any)
				delete(settings["PETG"].(map[string]any), "nozzle_temperature")
			})
			if err == nil || bad["ok"] != false || !strings.Contains(fmt.Sprint(bad["error"]), "nozzle_temperature") {
				t.Fatalf("missing installed PETG temperature was accepted: err=%v reply=%v", err, bad)
			}
			bad, err = invoke(func(bundle map[string]any) {
				profile := bundle["profile"].(map[string]any)
				project := profile["project_settings"].(map[string]any)
				project["nozzle_diameter"] = []any{"0.4", "0.6"}
			})
			if err == nil || bad["ok"] != false || !strings.Contains(fmt.Sprint(bad["error"]), "mixed nozzle") {
				t.Fatalf("mixed physical nozzles accepted: err=%v reply=%v", err, bad)
			}
			after, err := os.ReadFile(source)
			if err != nil || before != sha256.Sum256(after) {
				t.Fatal("approved installed OEM snapshot was modified")
			}
			t.Logf("RUST_INSTALLED_OEM_PREFLIGHT_PASS target=%s model=%s virtual_materials=5 source_sha256=%x", tc.target, tc.model, before)
		})
	}
}
