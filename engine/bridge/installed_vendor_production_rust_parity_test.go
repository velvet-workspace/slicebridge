package bridge

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"
)

// TestRustInstalledOEMProductionPlanParity checks the *actual production Go
// ConvertTarget planning/publishing path*, including installed native material
// binding, before comparing the complete experimental Rust output ZIP.
// Previously the OEM Rust parity fixture used machine-only project settings,
// which did not test material temperatures/flow/selected native profile IDs.
//
// This opt-in gate reads only copied installed OEM preset snapshots in a
// caller-specified scratch directory. It writes only to t.TempDir and an
// explicitly selected exclusive-create artifact export, never installed
// slicer presets or printer commands.
func TestRustInstalledOEMProductionPlanParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if rust == "" || root == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE and approved SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	}
	for _, target := range []string{"u1", "qidi"} {
		for _, sourceKind := range []string{
			"bambu-one-color.3mf",
			"bambu-five-colors.3mf",
			"mixed-five.3mf",
		} {
			t.Run(target+"_"+sourceKind, func(t *testing.T) {
				bundlePath := filepath.Join(root, fmt.Sprintf("scratch-%s-0.4-resolved.json", target))
				bundle, err := loadTargetBundle(bundlePath, target)
				if err != nil {
					t.Fatalf("installed OEM identity/nozzle validation failed: %v", err)
				}
				if bundle.Nozzle != "0.4" {
					t.Fatal("test requires exactly installed 0.4mm OEM nozzle")
				}

				dir := t.TempDir()
				source := filepath.Join(dir, sourceKind)
				if sourceKind == "mixed-five.3mf" {
					fixture(t, source, 5)
					mutateFiveSlotSource(t, source, false, "PETG")
				} else {
					raw, err := os.ReadFile(filepath.Join("..", "..", "tests", "fixtures", sourceKind))
					if err != nil {
						t.Fatal(err)
					}
					if err := os.WriteFile(source, raw, 0600); err != nil {
						t.Fatal(err)
					}
				}
				sourceBefore, err := os.ReadFile(source)
				if err != nil {
					t.Fatal(err)
				}
				goOutput := filepath.Join(dir, "go-bound-final.3mf")
				rustOutput := filepath.Join(dir, "rust-bound-final.3mf")

				// Exercise exactly the same Go target resolution and material
				// binding as the real active application conversion path.
				result, err := ConvertTarget(source, goOutput, TargetRequest{
					Target: target, ProfileFile: bundlePath, Nozzle: "0.4",
				})
				if target == "u1" && sourceKind == "mixed-five.3mf" {
					// The observed installed U1 PETG OEM snapshot has a
					// nozzle_temperature_range_high but lacks the safety-
					// critical lower bound. The real Go planner MUST refuse
					// mixing it with a PLA preset that declares that field.
					// Do not auto-fill a PETG temperature from a PLA default.
					if _, available := bundle.MaterialSettings["PETG"]["nozzle_temperature_range_low"]; available {
						t.Fatal("U1 OEM PETG snapshot changed: revalidate full positive material binding before updating this gate")
					}
					if err == nil || !strings.Contains(err.Error(), "nozzle_temperature_range_low") {
						t.Fatalf("incomplete installed PETG safety range must fail closed: result=%v error=%v", result, err)
					}
					if _, statErr := os.Stat(goOutput); !os.IsNotExist(statErr) {
						t.Fatal("refused native PETG source still published a Go output")
					}
					current, readErr := os.ReadFile(source)
					if readErr != nil || !bytes.Equal(sourceBefore, current) {
						t.Fatal("rejected native PETG conversion modified source")
					}
					t.Log("EXPECTED SAFE REJECTION: installed Snapmaker U1 PETG material profile lacks nozzle_temperature_range_low; no 3MF output published")
					return
				}
				if err != nil {
					t.Fatalf("Go production target conversion: %v", err)
				}
				if result["target"] != target ||
					result["target_nozzle"] != "0.4" ||
					result["profile_kind"] != string(ProfileInstalledVendor) {
					t.Fatalf("actual Go conversion used unexpected printer: %+v", result)
				}

				plan, err := buildPlan(source, filepath.Join(dir, "unused-plan.3mf"),
					func(src NozzleInfo) (*ResolvedTargetProfile, error) {
						return resolveBuiltinTarget(target, bundlePath, "0.4", src.Nozzle)
					})
				if err != nil {
					t.Fatalf("Go production plan is not reproducible: %v", err)
				}
				if plan.expectedModel != bundle.Profile.ModelID || plan.targetNozzle != "0.4" {
					t.Fatalf("reconstructed Go plan has wrong target identity: %+v", plan)
				}
				merged, preserved, err := preservedProfile(source, plan.base)
				if err != nil {
					t.Fatal(err)
				}
				if err := merged.Validate(); err != nil {
					t.Fatal(err)
				}
				// Read Go's actual converted settings for a reliable SupportsAuto
				// result. This is only used to derive the Rust test request.
				var goSettings map[string]any
				if err := json.Unmarshal(
					inspectZipEntries(t, goOutput)["Metadata/project_settings.config"], &goSettings,
				); err != nil {
					t.Fatal(err)
				}

				// Source snapshot must come from original 3MF, not the Go output.
				var sourceSettings map[string]any
				if err := json.Unmarshal(
					inspectZipEntries(t, source)["Metadata/project_settings.config"], &sourceSettings,
				); err != nil {
					t.Fatal(err)
				}
				mapping := make(map[string]int, len(plan.mapping))
				for src, dst := range plan.mapping {
					mapping[strconv.Itoa(src)] = dst
				}
				supports := fmt.Sprint(goSettings["enable_support"]) == "1"
				explicit := map[string]any{
					"profile":         merged,
					"slots":           plan.slots,
					"slot_count":      merged.FilamentSlots,
					"mapping":         mapping,
					"source_settings": sourceSettings,
					"supports":        supports,
					"preserved":       preserved,
				}
				raw, err := json.Marshal(explicit)
				if err != nil {
					t.Fatal(err)
				}
				request := filepath.Join(dir, "verified-go-plan.json")
				if err := os.WriteFile(request, raw, 0600); err != nil {
					t.Fatal(err)
				}
				ctx, cancel := context.WithTimeout(context.Background(), 35*time.Second)
				defer cancel()
				stdout, runErr := exec.CommandContext(ctx, rust,
					"--convert-archive", request, source, rustOutput).Output()
				if ctx.Err() != nil {
					t.Fatal(ctx.Err())
				}
				var reply struct {
					OK           bool   `json:"ok"`
					Removed      int    `json:"removed"`
					Experimental bool   `json:"experimental"`
					Error        string `json:"error"`
				}
				if err := json.Unmarshal(stdout, &reply); err != nil {
					t.Fatalf("Rust returned invalid JSON: %v %s", err, stdout)
				}
				if runErr != nil || !reply.OK || !reply.Experimental {
					t.Fatalf("Rust refused Go-validated OEM material binding: command=%v response=%+v", runErr, reply)
				}
				if expected, ok := result["removed_slice_cache_entries"].(int); ok && reply.Removed != expected {
					t.Fatalf("Go/Rust stale slice removal count differs: Go=%d Rust=%d", expected, reply.Removed)
				}
				compareConverted3MFParts(t, goOutput, rustOutput)

				// Confirm selected material constraints really arrived from the
				// installed target's native material presets, rather than
				// silently falling back to a generic PLA template.
				filaments := plan.filaments
				for i, f := range filaments {
					for key, expected := range map[string]any{
						"filament_type":        f.Type,
						"filament_settings_id": merged.FilamentProfiles[f.Type],
						"filament_colour":      f.Color,
					} {
						values, ok := goSettings[key].([]any)
						if !ok || len(values) <= i || !reflect.DeepEqual(values[i], expected) {
							t.Fatalf("%s material slot %d differs from source/installed %s: got %v expected %v",
								target, i, key, goSettings[key], expected)
						}
					}
					preset, exists := bundle.MaterialSettings[f.Type]
					if !exists {
						t.Fatalf("missing installed %s preset %s", target, f.Type)
					}
					for _, key := range []string{
						"nozzle_temperature", "filament_flow_ratio", "filament_max_volumetric_speed",
					} {
						values, exists := preset[key].([]any)
						if !exists || len(values) == 0 {
							continue
						}
						got, ok := goSettings[key].([]any)
						if !ok || len(got) <= i || !reflect.DeepEqual(got[i], values[0]) {
							t.Fatalf("%s material slot %d key %s not sourced from installed OEM preset: got %v want %v",
								target, i, key, goSettings[key], values[0])
						}
					}
				}
				for _, key := range []string{
					"printer_model", "printer_settings_id", "print_settings_id",
					"nozzle_diameter", "machine_start_gcode", "machine_end_gcode",
					"change_filament_gcode",
				} {
					want, exists := bundle.Profile.ProjectSettings[key]
					if !exists {
						t.Fatalf("OEM source profile missing machine key %s", key)
					}
					if !reflect.DeepEqual(goSettings[key], want) {
						t.Fatalf("%s material binding modified machine-owned %s", target, key)
					}
				}
				gotSource, err := os.ReadFile(source)
				if err != nil || !bytes.Equal(gotSource, sourceBefore) {
					t.Fatalf("source project modified in %s/%s", target, sourceKind)
				}
				if inspectPackageFacts, err := inspectPackage(rustOutput); err != nil ||
					inspectPackageFacts.sliceCaches != 0 {
					t.Fatalf("Rust OEM output left unverified cache or invalid package: %v", err)
				}
				if outputDir := os.Getenv("SLICEBRIDGE_BOUND_OEM_OUTPUT_DIR"); outputDir != "" {
					if st, err := os.Stat(outputDir); err != nil || !st.IsDir() {
						t.Fatalf("explicit material-bound OEM output directory unavailable: %v", err)
					}
					for _, entry := range [][2]string{
						{goOutput, "bound-go-" + target + "-" + sourceKind},
						{rustOutput, "bound-rust-" + target + "-" + sourceKind},
					} {
						exportSlicerFixtureNoOverwrite(t, entry[0], filepath.Join(outputDir, entry[1]))
					}
				}
				t.Logf("Go production plan and Rust ZIP parity with installed %s profile, %d source materials, %d output slots; OEM material temperature/flow/profile IDs and machine identity verified",
					target, len(filaments), merged.FilamentSlots)
			})
		}
	}
}
