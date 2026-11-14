package bridge

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
)

// This opt-in test compares the first independently Rust-planned native OEM
// conversion receipt against Go's real buildPlan, using approved, flattened
// installed-vendor snapshots. No slicer, user preset, printer or G-code runs.
func TestRustInstalledOEMPlanReceiptParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	root := os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if rust == "" || root == "" {
		t.Skip("requires explicit Rust binary and approved installed OEM snapshots")
	}
	source := filepath.Join("..", "..", "tests", "fixtures", "bambu-five-colors.3mf")
	insp, err := converter.Inspect(source)
	if err != nil {
		t.Fatal(err)
	}
	settings, err := readRustSourceSettings(source)
	if err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{"u1", "qidi"} {
		t.Run(target, func(t *testing.T) {
			snapshot := filepath.Join(root, "scratch-"+target+"-0.4-resolved.json")
			raw, err := os.ReadFile(snapshot)
			if err != nil {
				t.Fatal(err)
			}
			var bundle map[string]any
			if err := json.Unmarshal(raw, &bundle); err != nil {
				t.Fatal(err)
			}
			request := map[string]any{
				"bundle": bundle, "filaments": insp.Filaments,
				"source_settings": settings, "preserved": []string{},
			}
			reply, runErr := runRustMaterialCommand(t, rust, "--plan-native-conversion", request)
			if runErr != nil || reply["ok"] != true || reply["experimental"] != true {
				t.Fatalf("verified Rust OEM planner rejected installed %s: %v %v", target, runErr, reply)
			}
			plan, err := buildPlan(source, filepath.Join(t.TempDir(), "go-output.3mf"), func(src NozzleInfo) (*ResolvedTargetProfile, error) {
				return resolveBuiltinTarget(target, snapshot, "0.4", src.Nozzle)
			})
			if err != nil {
				t.Fatalf("Go OEM plan refused same input: %v", err)
			}
			mapping := make(map[string]int, len(plan.mapping))
			for old, slot := range plan.mapping {
				mapping[strconv.Itoa(old)] = slot
			}
			expected := map[string]any{
				"profile_name":     plan.base.Name,
				"printer_model_id": plan.base.ModelID,
				"slot_count":       plan.base.FilamentSlots,
				"slots":            plan.slots,
				"mapping":          mapping,
				"supports":         sourceSupportsForRust(settings),
				"preserved":        []string{},
			}
			actual, ok := reply["receipt"].(map[string]any)
			if !ok {
				t.Fatalf("Rust planner omitted receipt: %v", reply)
			}
			parityJSON(t, expected, actual)
			for _, secret := range []string{"profile", "project_settings", "machine_start_gcode", "machine_end_gcode", "material_settings"} {
				if _, found := actual[secret]; found {
					t.Fatalf("native OEM CLI receipt exposed %s", secret)
				}
			}
			after, err := os.ReadFile(snapshot)
			if err != nil || !bytes.Equal(raw, after) {
				t.Fatalf("installed OEM snapshot was modified")
			}
			t.Logf("RUST_NATIVE_PLAN_OEM_RECEIPT_PARITY_PASS target=%s slots=%d", target, len(plan.slots))
		})
	}
}
