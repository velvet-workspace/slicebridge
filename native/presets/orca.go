package presets

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strconv"
	"strings"

	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// Installed Orca-family machine presets become conversion targets only when the
// machine, a compatible installed process and a PLA filament are locally present
// and explicitly list that machine as compatible. A broken vendor default does
// not hide another valid process for the same model. Nothing is inferred
// from vendor folder names, and no per-printer approval flag is consulted.

const maxOrcaRejections = 50
const maxOrcaTools = 8

// Hard bounds for one installed Orca-family scan. A vendor tree that exceeds
// them is rejected with a reason rather than read without limit.
const maxOrcaVendors = 64
const maxOrcaMachineFiles = 512
const maxOrcaCatalogFiles = 20000
const maxOrcaProfileBytes = 4 << 20
const maxOrcaDepth = 4

// errOrcaAbstract marks an inheritance-only machine preset. It is not a printer
// and is skipped silently rather than reported as a rejection.
var errOrcaAbstract = errors.New("abstract machine preset")

var orcaTargetPattern = regexp.MustCompile(`^orca-[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{10}$`)
var orcaMaterials = []string{"PLA", "PETG", "ABS", "ASA", "TPU"}
var orcaNozzles = []string{"0.2", "0.25", "0.4", "0.6", "0.8"}

// OrcaTarget is one installed machine that resolved completely. Its ID is
// derived from the machine file location and is re-resolved before each use.
type OrcaTarget struct {
	ID            string   `json:"id"`
	Title         string   `json:"title"`
	Slicer        string   `json:"slicer"`
	Vendor        string   `json:"vendor"`
	Machine       string   `json:"machine"`
	Model         string   `json:"model"`
	Nozzle        string   `json:"nozzle"`
	PhysicalTools int      `json:"physicalTools"`
	Materials     []string `json:"materials"`
}

// OrcaRejection explains why an installed machine preset is not offered.
type OrcaRejection struct {
	Vendor  string `json:"vendor"`
	Machine string `json:"machine"`
	Slicer  string `json:"slicer"`
	Reason  string `json:"reason"`
}

// OrcaDiscovery is the frontend view. Install paths never leave this package.
type OrcaDiscovery struct {
	Targets  []OrcaTarget    `json:"targets"`
	Rejected []OrcaRejection `json:"rejected"`
}

// OrcaBundle is the generic target bundle read by engine/bridge.
type OrcaBundle struct {
	Target           string                    `json:"target"`
	Vendor           string                    `json:"vendor"`
	Slicer           string                    `json:"slicer"`
	Nozzle           string                    `json:"nozzle"`
	PhysicalTools    int                       `json:"physical_tools"`
	Profile          printer.Profile           `json:"profile"`
	MaterialSettings map[string]map[string]any `json:"material_settings"`
}

type orcaCandidate struct {
	target OrcaTarget
	bundle OrcaBundle
	root   string // installed resource root that owns the machine file; never serialized
}

// IsOrcaTargetID reports whether an ID has the shape produced by this package.
func IsOrcaTargetID(id string) bool {
	return orcaTargetPattern.MatchString(id)
}

// DiscoverOrca lists installed machine presets that resolve completely, plus
// bounded reasons for every installed machine that does not.
func DiscoverOrca() OrcaDiscovery {
	return discoverOrca(installedOrcaLocations())
}

// ResolveOrcaBundle re-resolves an ID from disk. The ID is matched only against
// freshly discovered targets; it is never treated as a path.
func ResolveOrcaBundle(id string) (*OrcaBundle, error) {
	return resolveOrca(installedOrcaLocations(), id)
}

// ResolveOrcaExecutable returns the trusted app *owning the exact installed
// profile*, rather than accepting an executable path (or vendor name) from UI.
// It re-resolves the ID and refuses custom profile-only roots and links.
func ResolveOrcaExecutable(id string) (executable, slicerName string, err error) {
	return resolveOrcaExecutableAt(installedOrcaLocations(), id, runtime.GOOS)
}

func resolveOrcaExecutableAt(locations []slicerLocation, id, platform string) (string, string, error) {
	candidate, err := findOrcaCandidate(locations, id)
	if err != nil {
		return "", "", err
	}
	for _, loc := range locations {
		if loc.root != candidate.root || loc.name != candidate.target.Slicer {
			continue
		}
		if loc.executable == "" {
			return "", "", fmt.Errorf("installed profile has no trusted slicer application to open")
		}
		executable := loc.executable
		appDir := executable
		switch platform {
		case "windows":
			appDir = filepath.Dir(executable)
			// Some QIDIStudio builds ship one of two verified executable names.
			executable = qidiStudioExecutable(loc)
		case "darwin":
			// open -a accepts the .app bundle path, not its inner binary.
		default:
			return "", "", fmt.Errorf("opening Orca profiles is unsupported on %s", platform)
		}
		// Resolve the original app/profile relationship again at launch time.
		if !orcaChainIsReal(appDir, loc.root) {
			return "", "", fmt.Errorf("installed slicer path is missing or contains a link")
		}
		info, err := os.Lstat(executable)
		if err != nil {
			return "", "", fmt.Errorf("installed %s application is missing: %w", loc.name, err)
		}
		if platform == "windows" && (!info.Mode().IsRegular() || info.Size() == 0) {
			return "", "", fmt.Errorf("installed %s executable is not a regular non-empty file", loc.name)
		}
		if platform == "darwin" && !info.IsDir() {
			return "", "", fmt.Errorf("installed %s application is not a real .app directory", loc.name)
		}
		return executable, loc.name, nil
	}
	return "", "", fmt.Errorf("installed slicer application for profile is unavailable")
}

// installedOrcaLocations returns the trusted roots to scan, in priority order.
// An application folder is used only when every folder down to its profiles is a
// real directory. Explicit local overrides carry no executable and are used as given.
func installedOrcaLocations() []slicerLocation {
	locations := []slicerLocation{}
	for _, loc := range installedOrcaFamilyLocations() {
		if loc.executable != "" && !orcaAppIsReal(loc) {
			continue
		}
		locations = append(locations, loc)
	}
	return locations
}

// orcaAppIsReal checks the application folder chain above a profiles root.
func orcaAppIsReal(loc slicerLocation) bool {
	appDir := loc.executable
	if runtime.GOOS == "windows" {
		appDir = filepath.Dir(loc.executable)
	}
	return orcaChainIsReal(appDir, loc.root)
}

// orcaChainIsReal reports whether appDir and every folder down to root are real
// directories. A link anywhere on that chain could redirect the scan elsewhere.
func orcaChainIsReal(appDir, root string) bool {
	rel, err := filepath.Rel(appDir, root)
	if err != nil || rel == "." || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return false
	}
	current := appDir
	for _, part := range append([]string{""}, strings.Split(rel, string(filepath.Separator))...) {
		current = filepath.Join(current, part)
		info, err := os.Lstat(current)
		if err != nil || !info.IsDir() {
			return false
		}
	}
	return true
}

func discoverOrca(locations []slicerLocation) OrcaDiscovery {
	candidates, rejected := scanOrca(locations)
	discovery := OrcaDiscovery{Targets: []OrcaTarget{}, Rejected: []OrcaRejection{}}
	for _, candidate := range candidates {
		discovery.Targets = append(discovery.Targets, candidate.target)
	}
	sort.Slice(discovery.Targets, func(i, j int) bool {
		left, right := discovery.Targets[i], discovery.Targets[j]
		return left.Vendor+" "+left.Machine+" "+left.ID < right.Vendor+" "+right.Machine+" "+right.ID
	})
	if len(rejected) > maxOrcaRejections {
		rejected = rejected[:maxOrcaRejections]
	}
	discovery.Rejected = append(discovery.Rejected, rejected...)
	return discovery
}

// OrcaArtworkSource tells the backend where one re-resolved target's vendor
// resources live. It is never sent to the frontend and never accepted as input.
type OrcaArtworkSource struct {
	Root   string
	Vendor string
	Model  string
}

// ResolveOrcaArtworkSource re-resolves an ID from disk, exactly like
// ResolveOrcaBundle, and returns its installation root plus exact printer_model.
func ResolveOrcaArtworkSource(id string) (OrcaArtworkSource, error) {
	return resolveOrcaArtworkSource(installedOrcaLocations(), id)
}

func resolveOrcaArtworkSource(locations []slicerLocation, id string) (OrcaArtworkSource, error) {
	candidate, err := findOrcaCandidate(locations, id)
	if err != nil {
		return OrcaArtworkSource{}, err
	}
	return OrcaArtworkSource{
		Root: candidate.root, Vendor: candidate.target.Vendor, Model: candidate.target.Model,
	}, nil
}

func resolveOrca(locations []slicerLocation, id string) (*OrcaBundle, error) {
	candidate, err := findOrcaCandidate(locations, id)
	if err != nil {
		return nil, err
	}
	bundle := candidate.bundle
	return &bundle, nil
}

// findOrcaCandidate scans every location afresh and matches the ID exactly.
func findOrcaCandidate(locations []slicerLocation, id string) (orcaCandidate, error) {
	if !IsOrcaTargetID(id) {
		return orcaCandidate{}, fmt.Errorf("invalid installed profile identifier")
	}
	candidates, _ := scanOrca(locations)
	for _, candidate := range candidates {
		if candidate.target.ID == id {
			return candidate, nil
		}
	}
	return orcaCandidate{}, fmt.Errorf("installed profile is not available or no longer resolves completely")
}

// Vendor-branded Orca forks bundle upstream printer libraries for many other
// manufacturers. Finding an Anker preset under Snapmaker Orca (or a Flashforge
// preset under ElegooSlicer) does not make that fork the printer's intended
// conversion destination. Only the owner's profiles belong to a branded fork.
// Generic OrcaSlicer and explicitly supplied local profile roots remain
// multi-manufacturer, with their existing compatibility checks intact.
func orcaVendorOwnedByApp(appID, vendor string) bool {
	switch appID {
	case "snapmaker-orca":
		return strings.EqualFold(vendor, "Snapmaker")
	case "flash-studio":
		return strings.EqualFold(vendor, "Flashforge")
	case "elegoo-slicer":
		return strings.EqualFold(vendor, "Elegoo")
	case "qidi-studio":
		return strings.EqualFold(vendor, "QIDI") || strings.EqualFold(vendor, "X 5 Series")
	case "creality-print":
		return strings.EqualFold(vendor, "Creality")
	case "anycubic-slicer-next":
		return strings.EqualFold(vendor, "Anycubic")
	default:
		return true // Generic Orca and explicitly configured custom profile roots.
	}
}

// scanOrca walks the locations in priority order. A machine is offered once per
// vendor and printer name. Branded forks contribute ONLY their own manufacturer;
// for other makers, use a fully resolved generic OrcaSlicer installation instead.
func scanOrca(locations []slicerLocation) ([]orcaCandidate, []OrcaRejection) {
	candidates := []orcaCandidate{}
	rejected := []OrcaRejection{}
	offered := map[string]bool{}
	for _, loc := range locations {
		vendors, err := os.ReadDir(loc.root)
		if err != nil {
			continue
		}
		scannedVendors := 0
		for _, vendor := range vendors {
			// Bambu remains source-only. Upstream profiles bundled into a
			// different maker's fork are not offered as ready destinations.
			if !vendor.IsDir() || vendor.Name() == "BBL" || vendor.Name() == "OrcaFilamentLibrary" ||
				!orcaVendorOwnedByApp(loc.id, vendor.Name()) {
				continue
			}
			// Bound the number of actual eligible vendor directories, not
			// root JSON index files / artwork. Orca-derived installations can
			// contain >64 top-level non-directory files before a real vendor.
			if scannedVendors >= maxOrcaVendors {
				break
			}
			scannedVendors++
			found, problems := scanOrcaVendor(loc, vendor.Name())
			for _, candidate := range found {
				key := candidate.target.Vendor + "\x00" + candidate.target.Machine
				if offered[key] {
					continue
				}
				offered[key] = true
				candidates = append(candidates, candidate)
			}
			rejected = append(rejected, problems...)
		}
	}
	return candidates, rejected
}

// scanOrcaVendor reads one vendor's preset index once, then checks each of its
// machine files independently so one bad machine never hides the others.
func scanOrcaVendor(loc slicerLocation, vendor string) ([]orcaCandidate, []OrcaRejection) {
	machineDir := filepath.Join(loc.root, vendor, "machine")
	entries, err := os.ReadDir(machineDir)
	if err != nil {
		return nil, nil
	}
	files := []string{}
	for _, entry := range entries {
		// Regular files only: a link could point outside the installed profiles.
		if entry.Type().IsRegular() && strings.EqualFold(filepath.Ext(entry.Name()), ".json") {
			files = append(files, filepath.Join(machineDir, entry.Name()))
		}
	}
	if len(files) == 0 {
		return nil, nil
	}
	if len(files) > maxOrcaMachineFiles {
		return nil, []OrcaRejection{{Vendor: vendor, Slicer: loc.name, Reason: "too many machine presets to scan"}}
	}
	index, err := readOrcaCatalog(loc.root, vendor)
	if err != nil {
		return nil, []OrcaRejection{{Vendor: vendor, Slicer: loc.name, Reason: "vendor profiles unreadable: " + err.Error()}}
	}
	byMachine := orcaFilamentsByMachine(index)
	byProcess := orcaProcessesByMachine(index)
	candidates := []orcaCandidate{}
	rejected := []OrcaRejection{}
	for _, file := range files {
		candidate, err := resolveOrcaMachine(index, byMachine, byProcess, loc, vendor, file)
		if errors.Is(err, errOrcaAbstract) {
			continue
		}
		if err != nil {
			rejected = append(rejected, OrcaRejection{
				Vendor: vendor, Machine: strings.TrimSuffix(filepath.Base(file), ".json"),
				Slicer: loc.name, Reason: err.Error(),
			})
			continue
		}
		candidates = append(candidates, candidate)
	}
	return candidates, rejected
}

// resolveOrcaMachine applies every fail-closed check to one machine file.
func resolveOrcaMachine(index catalog, byMachine map[string]map[string][]string, byProcess map[string][]string, loc slicerLocation, vendor, file string) (orcaCandidate, error) {
	stem := strings.TrimSuffix(filepath.Base(file), ".json")
	data, err := os.ReadFile(file)
	if err != nil {
		return orcaCandidate{}, err
	}
	var leaf map[string]any
	if err := unmarshalVendorPresetJSON(data, &leaf); err != nil {
		return orcaCandidate{}, fmt.Errorf("machine JSON is invalid")
	}
	if fmt.Sprint(leaf["instantiation"]) == "false" {
		return orcaCandidate{}, errOrcaAbstract
	}
	machineName, _ := leaf["name"].(string)
	if machineName == "" {
		machineName = stem
	}
	machine, err := flatten(index, "machine", stem, map[string]bool{})
	if err != nil {
		return orcaCandidate{}, err
	}
	if technology := orcaFirstString(machine["printer_technology"]); technology != "" && technology != "FFF" {
		return orcaCandidate{}, fmt.Errorf("printer technology %s is not supported", technology)
	}
	model := orcaFirstString(machine["printer_model"])
	if model == "" {
		return orcaCandidate{}, fmt.Errorf("printer_model is missing")
	}
	for _, spec := range Specs {
		if spec.Model == model {
			return orcaCandidate{}, fmt.Errorf("%s is already offered by a built-in engine", model)
		}
	}
	nozzle, tools, err := orcaNozzle(machine["nozzle_diameter"])
	if err != nil {
		return orcaCandidate{}, err
	}
	for _, key := range []string{"machine_start_gcode", "machine_end_gcode"} {
		if _, ok := machine[key].(string); !ok {
			return orcaCandidate{}, fmt.Errorf("%s is missing", key)
		}
	}
	// Vendor machine defaults sometimes point to a different model's process
	// (observed in the user's installed Flash Studio, ELEGOO and Snapmaker Orca).
	// Resolve an explicitly compatible installed process instead of dropping
	// the printer. Never fall back to a guessed or incompatible process.
	processName, process, err := orcaCompatibleProcess(
		index, byProcess[machineName], orcaFirstString(machine["default_print_profile"]), machineName, nozzle)
	if err != nil {
		return orcaCandidate{}, err
	}
	materials := map[string]map[string]any{}
	filamentNames := map[string]string{}
	for _, material := range orcaMaterials {
		name := orcaPreferredChoice(byMachine[machineName][material], machine["default_filament_profile"])
		if name == "" {
			if material == "PLA" {
				return orcaCandidate{}, fmt.Errorf("no PLA filament preset lists this machine as compatible")
			}
			continue
		}
		filament, err := flatten(index, "filament", name, map[string]bool{})
		if err != nil {
			return orcaCandidate{}, err
		}
		materials[material] = filament
		filamentNames[material] = name
	}
	project := map[string]any{}
	for _, layer := range []map[string]any{machine, process, materials["PLA"]} {
		for key, value := range layer {
			project[key] = value
		}
	}
	// Firmware and hardware values belong to the installed machine.
	for key, value := range machine {
		if strings.HasPrefix(key, "machine_") || key == "nozzle_diameter" || key == "change_filament_gcode" ||
			key == "layer_change_gcode" || key == "before_layer_change_gcode" {
			project[key] = value
		}
	}
	project["printer_model"] = model
	project["printer_settings_id"] = machineName
	project["print_settings_id"] = processName
	project["filament_settings_id"] = []string{filamentNames["PLA"]}
	project["filament_type"] = []string{"PLA"}
	project["filament_colour"] = []string{"#FFFFFF"}

	target := orcaTargetID(vendor, machineName, file)
	materialNames := []string{}
	for _, material := range orcaMaterials {
		if _, ok := filamentNames[material]; ok {
			materialNames = append(materialNames, material)
		}
	}
	bundle := OrcaBundle{
		Target: target, Vendor: vendor, Slicer: loc.name, Nozzle: nozzle, PhysicalTools: tools,
		Profile: printer.Profile{
			Name: target, DisplayName: model, ModelID: model, FilamentSlots: tools,
			FilamentProfiles: filamentNames, DefaultFilamentProfile: filamentNames["PLA"], ProjectSettings: project,
		},
		MaterialSettings: materials,
	}
	return orcaCandidate{
		target: OrcaTarget{
			ID: target, Title: machineName, Slicer: loc.name, Vendor: vendor, Machine: machineName, Model: model,
			Nozzle: nozzle, PhysicalTools: tools, Materials: materialNames,
		},
		bundle: bundle,
		root:   loc.root,
	}, nil
}

// readOrcaCatalog is the bounded counterpart of readCatalog for one vendor. It
// reads only machine, process and filament JSON, never follows links, stops at
// maxOrcaDepth folders, and fails once maxOrcaCatalogFiles is exceeded. Files
// over maxOrcaProfileBytes are skipped, so anything that depends on them is
// rejected as incomplete.
func readOrcaCatalog(root, vendor string) (catalog, error) {
	out := catalog{}
	dirs := []string{filepath.Join(root, vendor)}
	if vendor == "Flashforge" {
		dirs = append(dirs, filepath.Join(root, "OrcaFilamentLibrary"))
	}
	visited := 0
	for _, dir := range dirs {
		err := filepath.WalkDir(dir, func(file string, d fs.DirEntry, walkErr error) error {
			if walkErr != nil {
				if errors.Is(walkErr, fs.ErrNotExist) && file == dir {
					return nil
				}
				return walkErr
			}
			if d.Type()&fs.ModeSymlink != 0 {
				return nil
			}
			rel, err := filepath.Rel(dir, file)
			if err != nil {
				return err
			}
			if d.IsDir() {
				if strings.Count(rel, string(filepath.Separator))+1 > maxOrcaDepth {
					return fs.SkipDir
				}
				return nil
			}
			if !strings.EqualFold(filepath.Ext(file), ".json") {
				return nil
			}
			typ := kind(rel)
			if typ == "" {
				return nil
			}
			visited++
			if visited > maxOrcaCatalogFiles {
				return fmt.Errorf("vendor profile tree exceeds %d files", maxOrcaCatalogFiles)
			}
			info, err := d.Info()
			if err != nil {
				return err
			}
			if info.Size() > maxOrcaProfileBytes {
				return nil
			}
			data, err := os.ReadFile(file)
			if err != nil {
				return err
			}
			var settings map[string]any
			if err := unmarshalVendorPresetJSON(data, &settings); err != nil {
				// The path is left out so the install location never reaches the UI.
				return fmt.Errorf("vendor JSON is invalid")
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

// orcaProcessesByMachine indexes *explicit* vendor process compatibility once
// per vendor. Re-using this index avoids rescanning thousands of process files
// for each nozzle variant and makes recovery from broken defaults deterministic.
func orcaProcessesByMachine(index catalog) map[string][]string {
	result := map[string][]string{}
	for key := range index {
		if !strings.HasPrefix(key, "process:") {
			continue
		}
		name := strings.TrimPrefix(key, "process:")
		machines, ok := orcaCompatibility(index, "process", name)
		if !ok {
			continue
		}
		for _, machine := range machines {
			result[machine] = append(result[machine], name)
		}
	}
	for machine, choices := range result {
		sort.Strings(choices)
		unique := choices[:0]
		for _, choice := range choices {
			if len(unique) == 0 || unique[len(unique)-1] != choice {
				unique = append(unique, choice)
			}
		}
		result[machine] = unique
	}
	return result
}

// orcaCompatibleProcess prefers the installed machine's valid default, but
// honors alternative vendor process profiles when that default is missing,
// misspelled or belongs to a *different* printer. Alternatives must explicitly
// list the exact machine, flatten successfully, and have a sane layer height.
// The best ordinary layer height is around half the nozzle diameter.
func orcaCompatibleProcess(index catalog, compatible []string, preferred, machine, nozzle string) (string, map[string]any, error) {
	has := func(name string) bool {
		for _, choice := range compatible {
			if choice == name {
				return true
			}
		}
		return false
	}
	diameter, err := strconv.ParseFloat(nozzle, 64)
	if err != nil || diameter <= 0 {
		return "", nil, fmt.Errorf("invalid machine nozzle for process lookup")
	}
	if preferred != "" && has(preferred) {
		if process, err := flatten(index, "process", preferred, map[string]bool{}); err == nil && !orcaProcessTooNarrowForNozzle(process, diameter) {
			return preferred, process, nil
		}
	}
	bestName := ""
	var bestProcess map[string]any
	bestScore := math.Inf(1)
	for _, name := range compatible {
		if name == preferred {
			continue
		}
		process, err := flatten(index, "process", name, map[string]bool{})
		if err != nil || orcaProcessTooNarrowForNozzle(process, diameter) {
			continue
		}
		height, err := strconv.ParseFloat(orcaFirstString(process["layer_height"]), 64)
		if err != nil || height <= 0 || height > diameter*0.8 {
			continue
		}
		score := math.Abs(height - diameter*0.5)
		if !strings.Contains(strings.ToLower(name), "standard") {
			score += diameter
		}
		if score < bestScore || (score == bestScore && (bestName == "" || name < bestName)) {
			bestName, bestProcess, bestScore = name, process, score
		}
	}
	if bestName == "" {
		return "", nil, fmt.Errorf("no installed compatible process for %q (default %q unusable)", machine, preferred)
	}
	return bestName, bestProcess, nil
}

// A few installed vendor defaults inherit a 0.2 mm process even for 0.6/0.8
// mm nozzles (Peopoly Magneto X). Orca 2.4.2 rejects the resulting 0.22 mm
// line as too small. Reject that process choice, not the vendor's values:
// a different explicitly compatible installed process must supply the width.
// An absent, automatic (0), percentage, or unparseable width is left alone.
func orcaProcessTooNarrowForNozzle(process map[string]any, diameter float64) bool {
	value := strings.TrimSpace(orcaFirstString(process["line_width"]))
	width, err := strconv.ParseFloat(value, 64)
	return err == nil && width > 0 && width < diameter*0.5
}

// orcaNozzle requires one uniform, supported diameter per installed tool. The
// tool count is the length of that installed nozzle array.
func orcaNozzle(raw any) (string, int, error) {
	entries, _ := raw.([]any)
	if len(entries) < 1 || len(entries) > maxOrcaTools {
		return "", 0, fmt.Errorf("nozzle_diameter must list 1..%d tools", maxOrcaTools)
	}
	nozzle := ""
	for _, entry := range entries {
		value, err := strconv.ParseFloat(strings.TrimSpace(fmt.Sprint(entry)), 64)
		if err != nil {
			return "", 0, fmt.Errorf("nozzle_diameter has a non-numeric entry")
		}
		match := ""
		for _, supported := range orcaNozzles {
			reference, _ := strconv.ParseFloat(supported, 64)
			if math.Abs(reference-value) < 0.000001 {
				match = supported
			}
		}
		if match == "" {
			return "", 0, fmt.Errorf("unsupported nozzle diameter %v mm", entry)
		}
		if nozzle != "" && match != nozzle {
			return "", 0, fmt.Errorf("mixed nozzle diameters are not supported")
		}
		nozzle = match
	}
	return nozzle, len(entries), nil
}

// orcaFilamentsByMachine groups filament presets by the machines that name
// them as compatible, then by resolved filament_type.
func orcaFilamentsByMachine(index catalog) map[string]map[string][]string {
	result := map[string]map[string][]string{}
	seen := map[string]bool{}
	for key := range index {
		if !strings.HasPrefix(key, "filament:") {
			continue
		}
		name := strings.TrimPrefix(key, "filament:")
		if seen[name] {
			continue
		}
		seen[name] = true
		machines, ok := orcaCompatibility(index, "filament", name)
		if !ok {
			continue
		}
		flat, err := flatten(index, "filament", name, map[string]bool{})
		if err != nil {
			continue
		}
		material := orcaFirstString(flat["filament_type"])
		if material == "" {
			continue
		}
		for _, machine := range machines {
			if result[machine] == nil {
				result[machine] = map[string][]string{}
			}
			result[machine][material] = append(result[machine][material], name)
		}
	}
	for _, byMaterial := range result {
		for material := range byMaterial {
			sort.Strings(byMaterial[material])
		}
	}
	return result
}

// orcaCompatibility returns the explicit compatible_printers list of a preset
// or its nearest ancestor. A missing list is never read as "all printers".
func orcaCompatibility(index catalog, typ, name string) ([]string, bool) {
	for depth := 0; depth < 8; depth++ {
		choices := index[typ+":"+name]
		if len(choices) == 0 {
			return nil, false
		}
		leaf := choose(choices, name).settings
		if raw, ok := leaf["compatible_printers"]; ok {
			list, isList := raw.([]any)
			if !isList {
				return nil, false
			}
			names := []string{}
			for _, item := range list {
				if s, isString := item.(string); isString {
					names = append(names, s)
				}
			}
			return names, true
		}
		parent, _ := leaf["inherits"].(string)
		if parent == "" {
			return nil, false
		}
		name = parent
	}
	return nil, false
}

func orcaListsMachine(index catalog, typ, name, machine string) bool {
	names, ok := orcaCompatibility(index, typ, name)
	if !ok {
		return false
	}
	for _, listed := range names {
		if listed == machine {
			return true
		}
	}
	return false
}

// orcaPreferredChoice prefers the machine's declared default filament for this
// material, then the first compatible sorted name. An empty result means the
// material is unavailable for this machine and will fail if a project uses it.
func orcaPreferredChoice(choices []string, defaults any) string {
	if len(choices) == 0 {
		return ""
	}
	var listed []any
	switch value := defaults.(type) {
	case []any:
		listed = value
	case string:
		listed = []any{value}
	}
	for _, item := range listed {
		s, _ := item.(string)
		for _, choice := range choices {
			if s != "" && choice == s {
				return s
			}
		}
	}
	return choices[0]
}

// choose mirrors flatten: prefer the file whose stem matches the requested name.
func choose(choices []preset, name string) preset {
	for _, candidate := range choices {
		if strings.TrimSuffix(filepath.Base(candidate.file), ".json") == name {
			return candidate
		}
	}
	return choices[0]
}

func orcaFirstString(raw any) string {
	switch value := raw.(type) {
	case string:
		return value
	case []any:
		if len(value) > 0 {
			s, _ := value[0].(string)
			return s
		}
	}
	return ""
}

// orcaTargetID hashes the machine file location so IDs stay unique per install
// without exposing the path to the frontend.
func orcaTargetID(vendor, machine, file string) string {
	sum := sha256.Sum256([]byte(file))
	return "orca-" + orcaSlug(vendor+" "+machine, 40) + "-" + hex.EncodeToString(sum[:])[:10]
}

func orcaSlug(value string, limit int) string {
	var b strings.Builder
	dash := true
	for _, r := range strings.ToLower(value) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
			dash = false
		} else if !dash {
			b.WriteByte('-')
			dash = true
		}
	}
	slug := strings.Trim(b.String(), "-")
	if len(slug) > limit {
		slug = strings.Trim(slug[:limit], "-")
	}
	if slug == "" {
		return "printer"
	}
	return slug
}
