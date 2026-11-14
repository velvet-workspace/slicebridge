package bridge

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
)

// Opt-in local QA only: native OEM source snapshots are read, while model,
// Go outputs, Rust outputs, and the temporary Rust plan are written into an
// explicitly named scratch directory via exclusive creation. No slicer,
// user profile, print job, G-code or physical printer is touched.
func TestQIDIFlashforgeMixedFiveCenteredRustParity(t *testing.T) {
	dir := os.Getenv("SLICEBRIDGE_MIXED_FIVE_OUTPUT_DIR")
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	qidiProfile := os.Getenv("SLICEBRIDGE_MIXED_QIDI_PROFILE")
	flashProfile := os.Getenv("SLICEBRIDGE_MIXED_FLASH_PROFILE")
	if dir == "" || rust == "" || qidiProfile == "" || flashProfile == "" {
		t.Skip("opt-in: provide output dir, compiled Rust and verified QIDI + Flashforge profile snapshots")
	}
	if info, err := os.Stat(dir); err != nil || !info.IsDir() {
		t.Fatal("explicit artifact directory is not an existing directory")
	}
	var flash struct {
		Target        string `json:"target"`
		Nozzle        string `json:"nozzle"`
		PhysicalTools int    `json:"physical_tools"`
	}
	bytesFlash, err := os.ReadFile(flashProfile)
	if err != nil || json.Unmarshal(bytesFlash, &flash) != nil {
		t.Fatal("cannot read verified installed Flashforge profile snapshot")
	}
	if flash.Target != "orca-flashforge-flashforge-creator-5-pro-0-4-bcb0e97ccb" ||
		flash.Nozzle != "0.4" || flash.PhysicalTools != 4 {
		t.Fatalf("Flashforge installed machine/nozzle identity changed: %+v", flash)
	}
	prototype := filepath.Join(t.TempDir(), "known-native-readable.3mf")
	createPinnedNativeReadableMixedFiveSource(t, prototype)
	type trial struct {
		label, target, profile, model string
		bed, tools                    int
		orca                          bool
	}
	for _, tc := range []trial{
		{"qidi-x-plus-5", "qidi", qidiProfile, "X-Plus 5", 320, 1, false},
		{"flashforge-creator-5-pro", flash.Target, flashProfile, "Flashforge Creator 5 Pro", 256, 4, true},
	} {
		t.Run(tc.label, func(t *testing.T) {
			// The tetrahedron occupies X/Y = 0..10mm before translation.
			// Center its bounding box, not its origin, in the verified
			// 320x320 (QIDI) / 256x256 (Flashforge) installed build regions.
			centerXY := (tc.bed - 10) / 2
			source := filepath.Join(dir, "source-"+tc.label+"-center-4PLA-1PETG.3mf")
			centeredFiveSource(t, prototype, source, centerXY)
			original, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			goOutput := filepath.Join(dir, "go-"+tc.label+"-center-4PLA-1PETG.3mf")
			rustOutput := filepath.Join(dir, "rust-"+tc.label+"-center-4PLA-1PETG.3mf")
			request := TargetRequest{Target: tc.target, ProfileFile: tc.profile, Nozzle: "0.4"}
			report, err := ConvertTarget(source, goOutput, request)
			if err != nil {
				t.Fatalf("Go production conversion rejected installed profile: %v", err)
			}
			for key, expected := range map[string]any{
				"target": tc.target, "target_nozzle": "0.4",
				"source_filament_slots": 5, "target_filament_slots_used": 5,
				"virtual_filament_slots": 5, "physical_toolheads": tc.tools,
				"user_slot_mapping_required": true, "tool_usage_verification_required": true,
			} {
				if report[key] != expected {
					t.Fatalf("Go contract %s: got %v expected %v", key, report[key], expected)
				}
			}
			plan, err := buildPlan(source, filepath.Join(t.TempDir(), "plan-output.3mf"), func(sourceNozzle NozzleInfo) (*ResolvedTargetProfile, error) {
				if tc.orca {
					return resolveInstalledOrca(tc.profile, tc.target, "0.4")
				}
				return resolveBuiltinTarget("qidi", tc.profile, "0.4", sourceNozzle.Nozzle)
			})
			if err != nil {
				t.Fatal(err)
			}
			if plan.expectedModel != tc.model || plan.physicalTools != tc.tools {
				t.Fatalf("plan diverged from installed printer hardware: %s/%d", plan.expectedModel, plan.physicalTools)
			}
			merged, preserved, err := preservedProfile(source, plan.base)
			if err != nil {
				t.Fatal(err)
			}
			if err := merged.Validate(); err != nil {
				t.Fatal(err)
			}
			sourceZip := inspectZipEntries(t, source)
			goZip := inspectZipEntries(t, goOutput)
			var sourceSettings, goSettings map[string]any
			if err := json.Unmarshal(sourceZip["Metadata/project_settings.config"], &sourceSettings); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(goZip["Metadata/project_settings.config"], &goSettings); err != nil {
				t.Fatal(err)
			}
			slotMap := map[string]int{}
			for from, to := range plan.mapping {
				slotMap[strconv.Itoa(from)] = to
			}
			planJSON, err := json.Marshal(map[string]any{
				"profile": merged, "slots": plan.slots,
				"slot_count": merged.FilamentSlots, "mapping": slotMap,
				"source_settings": sourceSettings,
				"supports":        fmt.Sprint(goSettings["enable_support"]) == "1",
				"preserved":       preserved,
			})
			if err != nil {
				t.Fatal(err)
			}
			planFile := filepath.Join(t.TempDir(), "resolved-go-plan.json")
			if err := os.WriteFile(planFile, planJSON, 0600); err != nil {
				t.Fatal(err)
			}
			stdout, runErr := exec.Command(rust, "--convert-archive", planFile, source, rustOutput).CombinedOutput()
			var reply struct {
				OK           bool   `json:"ok"`
				Experimental bool   `json:"experimental"`
				Error        string `json:"error"`
			}
			if err := json.Unmarshal(stdout, &reply); err != nil || runErr != nil || !reply.OK || !reply.Experimental {
				t.Fatalf("Rust conversion unsuccessful: reply=%+v parse=%v exec=%v stdout=%s", reply, err, runErr, stdout)
			}
			compareConverted3MFParts(t, goOutput, rustOutput)
			// Independent Go and Rust package settings and the original source
			// each preserve all five explicit source material identities.
			wantedTypes := []string{"PLA", "PLA", "PLA", "PLA", "PETG"}
			wantedColors := []string{"#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FFFFFF"}
			for _, output := range []string{goOutput, rustOutput} {
				zipParts := inspectZipEntries(t, output)
				var settings map[string]any
				if err := json.Unmarshal(zipParts["Metadata/project_settings.config"], &settings); err != nil {
					t.Fatal(err)
				}
				types, okType := settings["filament_type"].([]any)
				colors, okColor := settings["filament_colour"].([]any)
				profiles, okProfiles := settings["filament_settings_id"].([]any)
				if !okType || !okColor || !okProfiles || len(types) != 5 || len(colors) != 5 || len(profiles) != 5 {
					t.Fatalf("%s did not preserve all 5 complete material slots", output)
				}
				for i := 0; i < 5; i++ {
					if types[i] != wantedTypes[i] || colors[i] != wantedColors[i] || profiles[i] == "" {
						t.Fatalf("%s slot %d is incorrect: type=%v color=%v preset=%v", output, i+1, types[i], colors[i], profiles[i])
					}
				}
				expectedPosition := []byte(fmt.Sprintf(" %d %d 0", centerXY, centerXY))
				if !bytes.Contains(zipParts["3D/3dmodel.model"], expectedPosition) {
					t.Fatalf("%s did not preserve centered transform %s", output, expectedPosition)
				}
				if _, leaked := zipParts["Metadata/plate_1.gcode"]; leaked {
					t.Fatalf("%s unexpectedly has cached G-code", output)
				}
			}
			updated, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(updated, original) {
				t.Fatal("original centered source was modified")
			}
			t.Logf("PASS target=%s bed=%dx%dmm, model XY=%d..%dmm, material slots 4 PLA + 1 PETG, toolheads=%d; Go/Rust complete 3MF package parity", tc.model, tc.bed, tc.bed, centerXY, centerXY+10, tc.tools)
		})
	}
}

// Copies a prior independently verified synthetic source with the precise
// build transform changed to the middle of the actual target bed. This does
// not edit the mesh, paint tokens, source settings or installed OEM profiles.
func centeredFiveSource(t *testing.T, source, destination string, centerXY int) {
	t.Helper()
	archive, err := zip.OpenReader(source)
	if err != nil {
		t.Fatal(err)
	}
	defer archive.Close()
	f, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal(err)
	}
	writer := zip.NewWriter(f)
	centered := false
	for _, entry := range archive.File {
		stream, err := entry.Open()
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(stream)
		_ = stream.Close()
		if err != nil {
			t.Fatal(err)
		}
		if entry.Name == "3D/3dmodel.model" {
			needle := []byte("1 0 0 0 1 0 0 0 1 120 120 0")
			if bytes.Count(body, needle) != 1 {
				t.Fatal("pinned source transform changed")
			}
			newTransform := []byte(fmt.Sprintf("1 0 0 0 1 0 0 0 1 %d %d 0", centerXY, centerXY))
			body = bytes.Replace(body, needle, newTransform, 1)
			centered = true
		}
		header := entry.FileHeader
		dst, err := writer.CreateHeader(&header)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := dst.Write(body); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	if !centered {
		t.Fatal("missing expected 3MF model")
	}
}
