// Package presets reads installed slicer profiles on Windows and macOS.
package presets

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

type Bundle struct {
	Target           string                    `json:"target"`
	Slicer           string                    `json:"slicer"`
	Root             string                    `json:"root"`
	Nozzle           string                    `json:"nozzle"`
	Profile          printer.Profile           `json:"profile"`
	MaterialSettings map[string]map[string]any `json:"material_settings"`
}
type Availability struct {
	Target    string `json:"target"`
	Name      string `json:"name"`
	Slicer    string `json:"slicer"`
	Available bool   `json:"available"`
	Error     string `json:"error,omitempty"`
}
type Spec struct {
	Name, Model, Slicer, Vendor string
	Nozzles                     []string
}

var Specs = map[string]Spec{
	"u1":   {"Snapmaker U1", "Snapmaker U1", "Snapmaker Orca", "Snapmaker", []string{"0.2", "0.4", "0.6", "0.8"}},
	"c5":   {"Flashforge Creator 5", "Flashforge Creator 5", "Flash Studio Desktop", "Flashforge", []string{"0.25", "0.4", "0.6", "0.8"}},
	"cc2":  {"ELEGOO CC2", "Elegoo Centauri Carbon 2", "ElegooSlicer", "Elegoo", []string{"0.2", "0.4", "0.6", "0.8"}},
	"qidi": {"QIDI Plus5", "X-Plus 5", "OrcaSlicer / QIDIStudio", "X 5 Series", []string{"0.2", "0.4", "0.6", "0.8"}},
}

// Variant maps source nozzle to actual target nozzle and exact installed presets.
func Variant(target, source string) (string, string, string, map[string]string, error) {
	spec, ok := Specs[target]
	if !ok {
		return "", "", "", nil, fmt.Errorf("unknown target %s", target)
	}
	supports := func(nozzle string) bool {
		for _, n := range spec.Nozzles {
			if n == nozzle {
				return true
			}
		}
		return false
	}
	nozzle := source
	// 0.2 and 0.25 are interchangeable as *narrow nozzle categories*, not as
	// physical machine diameters. Resolve to the manufacturer's exact preset.
	if !supports(nozzle) {
		if source == "0.2" && supports("0.25") {
			nozzle = "0.25"
		} else if source == "0.25" && supports("0.2") {
			nozzle = "0.2"
		}
	}
	if !supports(nozzle) {
		return "", "", "", nil, fmt.Errorf("%s has no %s mm nozzle profile", spec.Name, source)
	}
	var machine, process string
	materials := map[string]string{}
	switch target {
	case "u1":
		machine = "Snapmaker U1 (" + nozzle + " nozzle)"
		process = map[string]string{"0.2": "0.12mm Standard @Snapmaker U1 (0.2 nozzle)", "0.4": "0.20mm Standard @Snapmaker U1 (0.4 nozzle)", "0.6": "0.30mm Standard @Snapmaker U1 (0.6 nozzle)", "0.8": "0.40mm Standard @Snapmaker U1 (0.8 nozzle)"}[nozzle]
		for _, m := range []string{"PLA", "PETG", "ABS", "ASA"} {
			materials[m] = "Generic " + m + " @U1 " + nozzle + " nozzle"
		}
		if nozzle == "0.6" || nozzle == "0.8" {
			materials["TPU"] = "Generic TPU @U1 " + nozzle + " nozzle"
		}
	case "c5":
		machine = "Flashforge Creator 5 " + nozzle + " nozzle"
		process = map[string]string{"0.25": "0.12mm Standard @FF C5 0.25 nozzle", "0.4": "0.20mm Standard @FF C5", "0.6": "0.30mm Standard @FF C5 0.6 nozzle", "0.8": "0.32mm Standard @FF C5 0.8 nozzle"}[nozzle]
		suffix := ""
		if nozzle == "0.25" {
			suffix = " 0.25 nozzle"
		}
		for _, m := range []string{"PLA", "PETG", "ABS", "ASA"} {
			materials[m] = "Flashforge " + m + " Basic @FF C5" + suffix
		}
	case "cc2":
		machine = "Elegoo Centauri Carbon 2 " + nozzle + " nozzle"
		process = map[string]string{"0.2": "0.10mm Standard @Elegoo CC2 0.2 nozzle", "0.4": "0.20mm Standard @Elegoo CC2 0.4 nozzle", "0.6": "0.30mm Standard @Elegoo CC2 0.6 nozzle", "0.8": "0.40mm Standard @Elegoo CC2 0.8 nozzle"}[nozzle]
		for _, m := range []string{"PLA", "PETG", "ABS", "ASA"} {
			materials[m] = "Elegoo " + m + " @ECC2"
		}
		materials["TPU"] = "Elegoo TPU 95A @ECC2"
	case "qidi":
		machine = "X-Plus 5 " + nozzle + " nozzle"
		process = map[string]string{"0.2": "0.10mm Standard @X-Plus 5 0.2 nozzle", "0.4": "0.20mm Standard @X-Plus 5", "0.6": "0.30mm Standard @X-Plus 5 0.6 nozzle", "0.8": "0.40mm Standard @X-Plus 5 0.8 nozzle"}[nozzle]
		for _, m := range []string{"PLA", "PETG", "ABS"} {
			materials[m] = "Generic " + m + " @Qidi X-Plus 5 " + nozzle + " nozzle"
		}
		if nozzle != "0.2" {
			materials["TPU"] = "Generic TPU 95A @Qidi X-Plus 5 " + nozzle + " nozzle"
		}
	}
	return nozzle, machine, process, materials, nil
}

// Candidates and ResolveSlicer use the same ordered location inventory.
func Candidates(target string) []string {
	paths := []string{}
	for _, loc := range slicerLocations(target) {
		paths = append(paths, loc.root)
	}
	return paths
}
func FindRoot(target string) (string, error) {
	spec, ok := Specs[target]
	if !ok {
		return "", fmt.Errorf("unknown target %q", target)
	}
	for _, root := range Candidates(target) {
		if info, err := os.Stat(filepath.Join(root, spec.Vendor)); err == nil && info.IsDir() {
			return root, nil
		}
	}
	return "", fmt.Errorf("%s installed profile data not found", spec.Slicer)
}

var omit = map[string]bool{
	"type": true, "name": true, "from": true, "inherits": true, "setting_id": true, "instantiation": true, "renamed_from": true,
	"compatible_printers": true, "compatible_printers_condition": true, "compatible_prints": true, "compatible_prints_condition": true,
	"upward_compatible_machine": true,
}

type preset struct {
	file     string
	settings map[string]any
}
type catalog map[string][]preset

func kind(file string) string {
	sep := string(os.PathSeparator)
	for _, k := range []string{"machine", "process", "filament"} {
		if strings.Contains(sep+file, sep+k+sep) {
			return k
		}
	}
	return ""
}

// unmarshalVendorPresetJSON tolerates the optional UTF-8 BOM emitted by
// some *installed* Orca-family vendor JSON resources (confirmed in the
// Snapmaker Orca U1 process chain). This is not a text-encoding conversion:
// only the exact leading EF BB BF signature is removed. Invalid JSON,
// UTF-16, partial BOMs and unexpected bytes still fail closed.
func unmarshalVendorPresetJSON(data []byte, target *map[string]any) error {
	return json.Unmarshal(bytes.TrimPrefix(data, []byte{0xEF, 0xBB, 0xBF}), target)
}

func readCatalog(root string, spec Spec) (catalog, error) {
	out := catalog{}
	dirs := []string{filepath.Join(root, spec.Vendor)}
	if spec.Vendor == "Flashforge" {
		dirs = append(dirs, filepath.Join(root, "OrcaFilamentLibrary"))
	}
	for _, dir := range dirs {
		err := filepath.WalkDir(dir, func(file string, d fs.DirEntry, entryError error) error {
			if entryError != nil {
				if errors.Is(entryError, os.ErrNotExist) && file == dir {
					return nil
				}
				return entryError
			}
			if d.IsDir() || !strings.EqualFold(filepath.Ext(file), ".json") {
				return nil
			}
			typ := kind(file)
			if typ == "" {
				return nil
			}
			data, err := os.ReadFile(file)
			if err != nil {
				return err
			}
			var settings map[string]any
			if err = unmarshalVendorPresetJSON(data, &settings); err != nil {
				return fmt.Errorf("vendor JSON %s: %w", file, err)
			}
			if len(settings) == 0 {
				return nil
			}
			stem := strings.TrimSuffix(filepath.Base(file), ".json")
			names := []string{stem}
			if s, ok := settings["name"].(string); ok && s != "" && s != stem {
				names = append(names, s)
			}
			for _, name := range names {
				key := typ + ":" + name
				out[key] = append(out[key], preset{file, settings})
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}
func flatten(ix catalog, typ, name string, stack map[string]bool) (map[string]any, error) {
	choices := ix[typ+":"+name]
	if len(choices) == 0 {
		return nil, fmt.Errorf("missing installed %s preset %q", typ, name)
	}
	cur := choices[0]
	for _, p := range choices {
		if strings.TrimSuffix(filepath.Base(p.file), ".json") == name {
			cur = p
			break
		}
	}
	if stack[cur.file] {
		return nil, fmt.Errorf("preset inheritance cycle %q", name)
	}
	stack[cur.file] = true
	defer delete(stack, cur.file)
	result := map[string]any{}
	if parent, ok := cur.settings["inherits"].(string); ok && parent != "" {
		base, err := flatten(ix, typ, parent, stack)
		if err != nil {
			return nil, err
		}
		result = base
	}
	for k, v := range cur.settings {
		if !omit[k] {
			result[k] = v
		}
	}
	return result, nil
}
func validDiameter(raw any, diameter string) bool {
	arr, ok := raw.([]any)
	if !ok || len(arr) == 0 {
		return false
	}
	expected, err := strconv.ParseFloat(diameter, 64)
	if err != nil {
		return false
	}
	for _, v := range arr {
		n, err := strconv.ParseFloat(fmt.Sprint(v), 64)
		if err != nil || n < expected-0.000001 || n > expected+0.000001 {
			return false
		}
	}
	return true
}
func Load(target, sourceNozzle string) (*Bundle, error) {
	return LoadConfigured(target, sourceNozzle, "auto", nil)
}

// installedU1PETG04Leaf selects only a fully verified installed OEM 0.4 mm
// Snapmaker U1 PETG *leaf*, not the abstract @U1 base. The upstream
// manufacturer data declares a compatible "Generic PETG" leaf with its
// own nozzle-temperature lower bound; the inherited base has only an upper
// bound and cannot safely be combined with another material.
func installedU1PETG04Leaf(index catalog, machineName string) string {
	const leafName = "Generic PETG"
	for _, entry := range index["filament:"+leafName] {
		if filepath.Base(entry.file) != leafName+".json" ||
			entry.settings["inherits"] != "Generic PETG @U1 base" {
			continue
		}
		compatible, ok := entry.settings["compatible_printers"].([]any)
		if !ok {
			continue
		}
		matched := false
		for _, name := range compatible {
			if value, ok := name.(string); ok && value == machineName {
				matched = true
				break
			}
		}
		if !matched {
			continue
		}
		flattened, err := flatten(index, "filament", leafName, map[string]bool{})
		if err != nil {
			continue
		}
		material, ok := flattened["filament_type"].([]any)
		if !ok || len(material) != 1 || fmt.Sprint(material[0]) != "PETG" {
			continue
		}
		lower, lowOK := flattened["nozzle_temperature_range_low"].([]any)
		upper, highOK := flattened["nozzle_temperature_range_high"].([]any)
		if !lowOK || !highOK || len(lower) != 1 || len(upper) != 1 {
			continue
		}
		low, lowErr := strconv.ParseFloat(fmt.Sprint(lower[0]), 64)
		high, highErr := strconv.ParseFloat(fmt.Sprint(upper[0]), 64)
		if lowErr != nil || highErr != nil || !(low > 0 && high > low && high <= 450) {
			continue
		}
		return leafName
	}
	return ""
}

// LoadConfigured uses only installed vendor presets, with verified saved
// physical nozzle, slicer and material overrides.
func LoadConfigured(target, sourceNozzle, slicerID string, overrides map[string]string) (*Bundle, error) {
	spec, ok := Specs[target]
	if !ok {
		return nil, fmt.Errorf("unknown printer %q", target)
	}
	// An "auto" choice must select the first COMPLETE vendor profile, not merely
	// the first installed app folder. If a manufacturer install has broken or
	// missing machine/process/PLA data, try the next trusted Orca-family source.
	// Explicit user selections NEVER fall back to another slicer silently.
	if slicerID == "" || slicerID == "auto" {
		seen, reasons := map[string]bool{}, []string{}
		for _, loc := range slicerLocations(target) {
			if seen[loc.id] {
				continue
			}
			seen[loc.id] = true
			bundle, loadErr := LoadConfigured(target, sourceNozzle, loc.id, overrides)
			if loadErr == nil {
				return bundle, nil
			}
			reasons = append(reasons, loc.name+": "+loadErr.Error())
		}
		if len(reasons) == 0 {
			return nil, fmt.Errorf("no installed %s profile source was found", spec.Name)
		}
		return nil, fmt.Errorf("no complete installed %s source profile: %s", spec.Name, strings.Join(reasons, "; "))
	}
	nozzle, machineName, processName, filamentNames, err := Variant(target, sourceNozzle)
	if err != nil {
		return nil, err
	}
	root, _, actualSlicer, err := ResolveSlicer(target, slicerID)
	if err != nil {
		return nil, err
	}
	if err := ValidateOverrides(target, nozzle, slicerID, overrides); err != nil {
		return nil, err
	}
	index, err := readCatalog(root, spec)
	if err != nil {
		return nil, err
	}
	get := func(k, n string) (map[string]any, error) { return flatten(index, k, n, map[string]bool{}) }
	machine, err := get("machine", machineName)
	if err != nil {
		return nil, err
	}
	if machine["printer_model"] != spec.Model {
		return nil, fmt.Errorf("unexpected machine model %v for %s", machine["printer_model"], machineName)
	}
	if !validDiameter(machine["nozzle_diameter"], nozzle) {
		return nil, fmt.Errorf("incorrect machine diameter, expected %s mm", nozzle)
	}
	process, err := get("process", processName)
	if err != nil {
		return nil, err
	}
	materials := map[string]map[string]any{}
	for m, defaultName := range filamentNames {
		name := defaultName
		override := overrides[m]
		if override != "" {
			name = override
		} else if target == "u1" && nozzle == "0.4" && m == "PETG" {
			// Use the installed OEM's exact 0.4-mm-compatible leaf only
			// when its provenance, inherited type and full temperature
			// range were independently checked. Never rewrite user presets.
			if validated := installedU1PETG04Leaf(index, machineName); validated != "" {
				name = validated
			}
		}
		preset, loadErr := get("filament", name)
		if loadErr != nil && override == "" {
			// Not all nozzle families have every material in every slicer
			// build. Never invent a missing vendor preset.
			alternatives, listErr := FilamentPresets(target, nozzle, slicerID, m)
			if listErr == nil && len(alternatives) > 0 {
				name = alternatives[0]
				preset, loadErr = get("filament", name)
			}
		}
		if loadErr != nil {
			if override != "" {
				return nil, loadErr
			}
			delete(filamentNames, m) // unsupported material fails only if used
			continue
		}
		raw, ok := preset["filament_type"].([]any)
		if !ok || len(raw) == 0 || fmt.Sprint(raw[0]) != m {
			if override != "" {
				return nil, fmt.Errorf("saved %s profile has incorrect material type", m)
			}
			delete(filamentNames, m)
			continue
		}
		materials[m] = preset
		filamentNames[m] = name
	}
	if materials["PLA"] == nil {
		return nil, fmt.Errorf("installed %s %s profile has no compatible PLA filament", target, nozzle)
	}
	project := map[string]any{}
	for _, layer := range []map[string]any{machine, process, materials["PLA"]} {
		for k, v := range layer {
			project[k] = v
		}
	}
	// Firmware and hardware values belong to the installed machine.
	for k, v := range machine {
		if strings.HasPrefix(k, "machine_") || k == "nozzle_diameter" || k == "change_filament_gcode" ||
			k == "layer_change_gcode" || k == "before_layer_change_gcode" {
			project[k] = v
		}
	}
	project["printer_model"] = spec.Model
	project["printer_settings_id"] = machineName
	project["print_settings_id"] = processName
	project["filament_settings_id"] = []string{filamentNames["PLA"]}
	project["filament_type"] = []string{"PLA"}
	project["filament_colour"] = []string{"#FFFFFF"}
	return &Bundle{
		Target: target, Slicer: actualSlicer, Root: root, Nozzle: nozzle,
		Profile: printer.Profile{
			Name: target, DisplayName: spec.Model, ModelID: spec.Model, FilamentSlots: 4,
			FilamentProfiles: filamentNames, DefaultFilamentProfile: filamentNames["PLA"], ProjectSettings: project,
		},
		MaterialSettings: materials,
	}, nil
}
func Available() []Availability {
	targets := []string{"u1", "c5", "cc2", "qidi"}
	out := make([]Availability, 0, len(targets))
	for _, t := range targets {
		spec := Specs[t]
		v := Availability{Target: t, Name: spec.Name, Slicer: spec.Slicer}
		if t == "u1" {
			v.Available = true
		} else {
			_, err := Load(t, "0.4")
			v.Available = err == nil
			if err != nil {
				v.Error = err.Error()
			}
		}
		out = append(out, v)
	}
	return out
}
