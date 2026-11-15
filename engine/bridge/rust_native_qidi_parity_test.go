package bridge

import (
	"reflect"
	"strconv"
	"strings"
	"testing"
)

// QIDIStudio X-Plus 5 project import requires a per-slot identity and color
// array. These are deliberate Rust-native 3MF compatibility corrections to
// the historical stable Go archive, not general permission to diverge from Go.
// All other project fields and ZIP parts remain exactly parity-checked.
func equivalentQIDINativeProjectFix(goSettings, rustSettings any) bool {
	original, ok := goSettings.(map[string]any)
	if !ok || original["printer_model"] != "X-Plus 5" {
		return false
	}
	native, ok := rustSettings.(map[string]any)
	if !ok {
		return false
	}
	types, ok := original["filament_type"].([]any)
	if !ok || len(types) == 0 || len(types) > 64 {
		return false
	}
	colors, ok := original["filament_colour"].([]any)
	if !ok || len(colors) != len(types) || !reflect.DeepEqual(colors, native["filament_colour"]) ||
		!reflect.DeepEqual(types, native["filament_type"]) {
		return false
	}
	ids, ok := native["filament_ids"].([]any)
	if !ok || len(ids) != len(types) {
		return false
	}
	index, ok := native["filament_self_index"].([]any)
	if !ok || len(index) != len(types) {
		return false
	}
	if !reflect.DeepEqual(native["filament_multi_colour"], colors) {
		return false
	}
	idByMaterial := make(map[string]string)
	materialByID := make(map[string]string)
	for i, material := range types {
		kind, ok := material.(string)
		if !ok || kind == "" {
			return false
		}
		id, ok := ids[i].(string)
		if !ok || id == "" || len(id) > 64 || strings.ContainsAny(id, " \t\r\n") {
			return false
		}
		if index[i] != strconv.Itoa(i+1) {
			return false
		}
		if known, found := idByMaterial[kind]; found && known != id {
			return false
		}
		if prior, found := materialByID[id]; found && prior != kind {
			return false
		}
		idByMaterial[kind] = id
		materialByID[id] = kind
	}
	withoutQIDIRepair := make(map[string]any, len(native))
	for key, value := range native {
		switch key {
		case "filament_ids", "filament_self_index", "filament_multi_colour":
			continue
		default:
			withoutQIDIRepair[key] = value
		}
	}
	// The stable Go archive carries an old single-material ID; Rust no longer
	// passes that scalar to a five-virtual-filament QIDIStudio project.
	if legacy, exists := original["filament_id"]; exists {
		if _, stillPresent := native["filament_id"]; stillPresent {
			return false
		}
		withoutQIDIRepair["filament_id"] = legacy
	}
	return reflect.DeepEqual(original, withoutQIDIRepair)
}

func TestQIDINativeProjectIdentityFixIsExactNotGeneralParityExemption(t *testing.T) {
	before := map[string]any{
		"printer_model":        "X-Plus 5",
		"filament_type":        []any{"PLA", "PETG"},
		"filament_colour":      []any{"#123456", "#FFFFFF"},
		"filament_settings_id": []any{"Generic PLA", "Generic PETG"},
		"filament_id":          "QFA90",
		"nozzle_temperature":   []any{"220", "250"},
	}
	good := map[string]any{
		"printer_model":         "X-Plus 5",
		"filament_type":         []any{"PLA", "PETG"},
		"filament_colour":       []any{"#123456", "#FFFFFF"},
		"filament_multi_colour": []any{"#123456", "#FFFFFF"},
		"filament_settings_id":  []any{"Generic PLA", "Generic PETG"},
		"filament_ids":          []any{"QFA90", "QFG90"},
		"filament_self_index":   []any{"1", "2"},
		"nozzle_temperature":    []any{"220", "250"},
	}
	if !equivalentQIDINativeProjectFix(before, good) {
		t.Fatal("real native per-slot material/color correction rejected")
	}
	for name, mutation := range map[string]func(map[string]any){
		"color changed":          func(v map[string]any) { v["filament_colour"] = []any{"#123456", "#060606"} },
		"original model changed": func(v map[string]any) { v["printer_model"] = "Other" },
		"material changed":       func(v map[string]any) { v["filament_type"] = []any{"PLA", "PLA"} },
		"material id reused":     func(v map[string]any) { v["filament_ids"] = []any{"QFA90", "QFA90"} },
		"color aliases disagree": func(v map[string]any) { v["filament_multi_colour"] = []any{"#123456", "#060606"} },
		"slot permutation":       func(v map[string]any) { v["filament_self_index"] = []any{"2", "1"} },
		"nozzle changed":         func(v map[string]any) { v["nozzle_temperature"] = []any{"220", "275"} },
		"legacy scalar retained": func(v map[string]any) { v["filament_id"] = "QFA90" },
	} {
		t.Run(name, func(t *testing.T) {
			altered := make(map[string]any, len(good))
			for key, value := range good {
				altered[key] = value
			}
			mutation(altered)
			if equivalentQIDINativeProjectFix(before, altered) {
				t.Fatal("disallowed vendor/material/color change accepted as parity")
			}
		})
	}
}
