package bridge

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// The bundle contains locally flattened *installed slicer* presets, not
// hand-authored firmware scripts. A target is never enabled without them.
type targetBundle struct {
	Target           string                    `json:"target"`
	Slicer           string                    `json:"slicer"`
	Root             string                    `json:"root"`
	Nozzle           string                    `json:"nozzle"`
	Profile          printer.Profile           `json:"profile"`
	MaterialSettings map[string]map[string]any `json:"material_settings"`
}

var targetModels = map[string]string{
	"u1":   "Snapmaker U1",
	"c5":   "Flashforge Creator 5",
	"cc2":  "Elegoo Centauri Carbon 2",
	"qidi": "X-Plus 5",
}

func validTargetNozzle(target, nozzle string) bool {
	allowed := map[string]map[string]bool{
		"u1":   {"0.2": true, "0.4": true, "0.6": true, "0.8": true},
		"c5":   {"0.25": true, "0.4": true, "0.6": true, "0.8": true},
		"cc2":  {"0.2": true, "0.4": true, "0.6": true, "0.8": true},
		"qidi": {"0.2": true, "0.4": true, "0.6": true, "0.8": true},
	}
	return allowed[target][nozzle]
}
func targetNozzleForSource(target, sourceNozzle string) (string, error) {
	if validTargetNozzle(target, sourceNozzle) {
		return sourceNozzle, nil
	}
	// The two *narrowest* manufacturer nozzle variants are one conversion
	// category, but output always uses the exact physical target profile.
	// For instance: Bambu 0.2 -> Flashforge 0.25, or source 0.25 -> U1 0.2.
	if sourceNozzle == "0.2" && validTargetNozzle(target, "0.25") {
		return "0.25", nil
	}
	if sourceNozzle == "0.25" && validTargetNozzle(target, "0.2") {
		return "0.2", nil
	}
	return "", fmt.Errorf("no matching %s target nozzle for source %s mm", target, sourceNozzle)
}
func verifyTargetBundleNozzle(bundle *targetBundle) error {
	raw := bundle.Profile.ProjectSettings["nozzle_diameter"]
	detected, err := nozzleFromRaw(raw)
	if err != nil || detected != bundle.Nozzle {
		return fmt.Errorf("target machine nozzle_diameter (%v) does not match bundled %s mm", raw, bundle.Nozzle)
	}
	return nil
}

func loadTargetBundle(filename, target string) (*targetBundle, error) {
	data, err := os.ReadFile(filename)
	if err != nil {
		return nil, err
	}
	if len(data) < 100 || len(data) > 4<<20 {
		return nil, fmt.Errorf("slicer profile bundle has invalid size")
	}
	var bundle targetBundle
	if err := json.Unmarshal(data, &bundle); err != nil {
		return nil, err
	}
	expected := targetModels[target]
	if expected == "" || bundle.Target != target || bundle.Profile.Name != target ||
		bundle.Profile.DisplayName != expected || bundle.Profile.ModelID != expected ||
		bundle.Profile.FilamentSlots != 4 || !validTargetNozzle(target, bundle.Nozzle) {
		return nil, fmt.Errorf("slicer bundle is not a supported %s target profile", target)
	}
	if err := bundle.Profile.Validate(); err != nil {
		return nil, err
	}
	if bundle.Profile.ProjectSettings["printer_model"] != expected {
		return nil, fmt.Errorf("target project settings do not match machine model")
	}
	if err := verifyTargetBundleNozzle(&bundle); err != nil {
		return nil, err
	}
	if _, ok := bundle.Profile.ProjectSettings["machine_start_gcode"].(string); !ok {
		return nil, fmt.Errorf("target firmware start script missing")
	}
	if _, ok := bundle.Profile.ProjectSettings["machine_end_gcode"].(string); !ok {
		return nil, fmt.Errorf("target firmware end script missing")
	}
	if len(bundle.Profile.FilamentProfiles) == 0 || len(bundle.MaterialSettings) == 0 {
		return nil, fmt.Errorf("target material presets missing")
	}
	return &bundle, nil
}

// Bind slicer's native filament parameters to every source material slot.
// The upstream converter handles colors, slot IDs and painted triangle refs.
func bindNativeFilamentSettings(bundle *targetBundle, filaments []converter.Filament) error {
	if len(filaments) == 0 || len(filaments) > bundle.Profile.FilamentSlots {
		return fmt.Errorf("invalid number of source filaments")
	}
	presets := make([]map[string]any, len(filaments))
	nativeProfileIDs := make([]any, len(filaments))
	for i, filament := range filaments {
		name := bundle.Profile.FilamentProfiles[filament.Type]
		preset := bundle.MaterialSettings[filament.Type]
		if name == "" || len(preset) == 0 {
			return fmt.Errorf("no installed %s filament preset for material type %q", bundle.Target, filament.Type)
		}
		presets[i] = preset
		nativeProfileIDs[i] = name
	}
	keys := make(map[string]bool)
	for _, preset := range presets {
		for key, value := range preset {
			if arr, ok := value.([]any); ok && len(arr) > 0 {
				keys[key] = true
			}
		}
	}
	names := make([]string, 0, len(keys))
	for key := range keys {
		names = append(names, key)
	}
	sort.Strings(names)
	for _, key := range names {
		// The installed machine's filament profile mapping is authoritative.
		// Raw filament JSON may omit filament_settings_id or contain an empty
		// placeholder, while Profile.FilamentProfiles provides the explicitly
		// resolved native preset for each material. Never infer a PLA fallback.
		if bundle.Target == "qidi" && key == "filament_settings_id" {
			continue
		}
		// Hardware-only settings such as nozzle diameter remain owned by
		// the *machine* profile, even if a material preset also sets them.
		if key == "nozzle_diameter" || key == "extruder_offset" ||
			key == "machine_max_acceleration_extruding" {
			continue
		}
		mapped := make([]any, len(presets))
		missing := false
		for i, preset := range presets {
			values, ok := preset[key].([]any)
			if !ok || len(values) == 0 {
				missing = true
				break
			}
			mapped[i] = values[0]
		}
		if missing {
			// Keep safety-critical target temperatures and flow limits fail-closed.
			// Optional fields are allowed to fall back to installed system
			// presets; never apply a PLA-only override to PETG/ABS slots.
			if strings.HasPrefix(key, "nozzle_temperature") ||
				strings.HasPrefix(key, "bed_temperature") ||
				key == "filament_max_volumetric_speed" ||
				strings.Contains(key, "flow_ratio") ||
				key == "filament_type" || key == "filament_settings_id" {
				return fmt.Errorf("missing critical native material setting %q", key)
			}
			delete(bundle.Profile.ProjectSettings, key)
			continue
		}
		bundle.Profile.ProjectSettings[key] = mapped
	}
	// The printer/print system IDs come from its installed slicer, whereas
	// these arrays are source slot-specific and must match exactly.
	bundle.Profile.ProjectSettings["filament_type"] = materialTypes(filaments)
	if bundle.Target == "qidi" {
		bundle.Profile.ProjectSettings["filament_settings_id"] = nativeProfileIDs
	}
	return nil
}
func materialTypes(filaments []converter.Filament) []any {
	out := make([]any, len(filaments))
	for i, fil := range filaments {
		out[i] = strings.TrimSpace(fil.Type)
	}
	return out
}
