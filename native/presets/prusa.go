package presets

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Installed PrusaSlicer printers become per-printer targets only when the user
// printer preset resolves completely: an FFF printer with a bed shape, one
// supported nozzle, and a compatible print and PLA filament preset found in the
// installed user folders or the vendor bundles. Compatibility follows
// PrusaSlicer's rules. A non-empty compatible_printers list decides alone.
// Otherwise a non-empty compatible_printers_condition decides. A preset with
// neither applies to every printer. Parent presets come only from the
// preset's own inherits list, and nothing is inferred from folder names.

const maxPrusaRejections = 50
const maxPrusaFilesPerDir = 512
const maxPrusaVendorFiles = 256
const maxPrusaFileBytes = 8 << 20
const maxPrusaTotalBytes = 128 << 20
const maxPrusaInherit = 8
const prusaSlicerLabel = "PrusaSlicer"
const prusaCustomVendor = "Custom"

var prusaTargetPattern = regexp.MustCompile(`^prusa-[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{10}$`)
var prusaNozzles = []string{"0.2", "0.25", "0.4", "0.6", "0.8"}
var errPrusaUnreadable = errors.New("profile file cannot be read safely")

// PrusaTarget is one installed user printer preset that resolved completely.
// Its ID is derived from the exact preset name and file location, and it is
// re-resolved from disk before any use. Install paths never leave this package.
type PrusaTarget struct {
	ID       string `json:"id"`
	Title    string `json:"title"`
	Slicer   string `json:"slicer"`
	Vendor   string `json:"vendor"`
	Model    string `json:"model"`
	Nozzle   string `json:"nozzle"`
	Print    string `json:"-"`
	Filament string `json:"-"`
}

// PrusaRejection explains why an installed printer preset is not offered.
type PrusaRejection struct {
	Vendor  string `json:"vendor"`
	Machine string `json:"machine"`
	Slicer  string `json:"slicer"`
	Reason  string `json:"reason"`
}

// PrusaDiscovery is the frontend view of installed Prusa printers.
type PrusaDiscovery struct {
	Targets  []PrusaTarget    `json:"targets"`
	Rejected []PrusaRejection `json:"rejected"`
}

// prusaLocations holds the trusted roots. user holds printer/, print/ and
// filament/ folders. vendor holds the bundled vendor .ini files.
type prusaLocations struct {
	user   string
	vendor string
}

type prusaProfile struct {
	file   string
	values prusaSection
}

// prusaSelection is the [presets] block of PrusaSlicer.ini: the printer last
// selected and the print and filament selected with it. It only ranks
// candidates and never adds one.
type prusaSelection struct {
	printer  string
	print    string
	filament string
}

type prusaPrinterSource struct {
	file string
	name string
}

// prusaIndex is built once per discovery. It is read-only afterwards.
type prusaIndex struct {
	profiles map[string][]prusaProfile // "kind:name" -> one entry per defining file
	vendors  map[string]string         // vendor file -> [vendor] name
	names    map[string][]string       // kind -> sorted selectable preset names
	printers []prusaPrinterSource      // user printer presets, in file order
	selected prusaSelection
}

// DiscoverPrusa lists installed user printers that resolve completely, plus
// bounded reasons for every user printer that does not.
func DiscoverPrusa() PrusaDiscovery {
	return discoverPrusa(installedPrusaLocations())
}

// IsPrusaTargetID reports whether an ID has the shape produced by this package.
func IsPrusaTargetID(id string) bool {
	return prusaTargetPattern.MatchString(id)
}

// ResolvePrusaTarget re-resolves an ID from disk. The ID is matched only against
// freshly discovered targets; it is never treated as a path.
func ResolvePrusaTarget(id string) (PrusaTarget, error) {
	return resolvePrusaTarget(installedPrusaLocations(), id)
}

func resolvePrusaTarget(loc prusaLocations, id string) (PrusaTarget, error) {
	if !IsPrusaTargetID(id) {
		return PrusaTarget{}, fmt.Errorf("invalid installed printer identifier")
	}
	for _, target := range discoverPrusa(loc).Targets {
		if target.ID == id {
			return target, nil
		}
	}
	return PrusaTarget{}, fmt.Errorf("installed printer profile is not available or no longer resolves completely")
}

func installedPrusaLocations() prusaLocations {
	loc := prusaLocations{}
	switch runtime.GOOS {
	case "windows":
		if appData := os.Getenv("APPDATA"); appData != "" {
			loc.user = filepath.Join(appData, "PrusaSlicer")
		}
		programFiles := os.Getenv("ProgramFiles")
		if programFiles == "" {
			programFiles = `C:\Program Files`
		}
		loc.vendor = filepath.Join(programFiles, "Prusa3D", "PrusaSlicer", "resources", "profiles")
	case "darwin":
		home, err := os.UserHomeDir()
		if err == nil {
			loc.user = filepath.Join(home, "Library", "Application Support", "PrusaSlicer")
		}
		loc.vendor = filepath.Join("/Applications", "PrusaSlicer.app", "Contents", "Resources", "profiles")
		if _, statErr := os.Stat(loc.vendor); statErr != nil && err == nil {
			loc.vendor = filepath.Join(home, "Applications", "PrusaSlicer.app", "Contents", "Resources", "profiles")
		}
	}
	return loc
}

func discoverPrusa(loc prusaLocations) PrusaDiscovery {
	index, problems := loadPrusaIndex(loc)
	discovery := PrusaDiscovery{Targets: []PrusaTarget{}, Rejected: []PrusaRejection{}}
	for _, source := range index.printers {
		target, err := index.resolvePrinter(source)
		if err != nil {
			problems = append(problems, PrusaRejection{
				Vendor: prusaCustomVendor, Machine: source.name, Slicer: prusaSlicerLabel, Reason: err.Error(),
			})
			continue
		}
		discovery.Targets = append(discovery.Targets, target)
	}
	sort.Slice(discovery.Targets, func(i, j int) bool {
		left, right := discovery.Targets[i], discovery.Targets[j]
		return left.Vendor+" "+left.Title+" "+left.ID < right.Vendor+" "+right.Title+" "+right.ID
	})
	if len(problems) > maxPrusaRejections {
		problems = problems[:maxPrusaRejections]
	}
	discovery.Rejected = append(discovery.Rejected, problems...)
	return discovery
}

// loadPrusaIndex reads vendor bundles first and then user folders. Any file that
// cannot be read or parsed is reported and skipped, so one bad file never hides
// the others.
func loadPrusaIndex(loc prusaLocations) (*prusaIndex, []PrusaRejection) {
	index := &prusaIndex{
		profiles: map[string][]prusaProfile{}, vendors: map[string]string{}, names: map[string][]string{},
	}
	problems := []PrusaRejection{}
	budget := int64(maxPrusaTotalBytes)
	load := func(files []string, user bool) {
		for _, file := range files {
			if budget <= 0 {
				problems = append(problems, PrusaRejection{Slicer: prusaSlicerLabel, Machine: filepath.Base(file), Reason: "profile folders exceed the size budget"})
				return
			}
			read, err := index.addFile(file, user, min(maxPrusaFileBytes, budget))
			if err != nil {
				problems = append(problems, PrusaRejection{Slicer: prusaSlicerLabel, Machine: strings.TrimSuffix(filepath.Base(file), filepath.Ext(file)), Reason: err.Error()})
			}
			budget -= read
		}
	}
	if loc.vendor != "" {
		files, err := listPrusaFiles(loc.vendor, maxPrusaVendorFiles)
		if err != nil && !errors.Is(err, fs.ErrNotExist) {
			problems = append(problems, PrusaRejection{Slicer: prusaSlicerLabel, Machine: "vendor bundles", Reason: "vendor folder cannot be scanned"})
		}
		load(files, false)
	}
	if loc.user != "" {
		for _, kind := range []string{"printer", "print", "filament"} {
			files, err := listPrusaFiles(filepath.Join(loc.user, kind), maxPrusaFilesPerDir)
			if err != nil && !errors.Is(err, fs.ErrNotExist) {
				problems = append(problems, PrusaRejection{Slicer: prusaSlicerLabel, Machine: kind + " folder", Reason: "user preset folder cannot be scanned"})
			}
			load(files, true)
		}
	}
	if loc.user != "" {
		index.selected = readPrusaSelection(filepath.Join(loc.user, "PrusaSlicer.ini"))
	}
	index.finish()
	return index, problems
}

// readPrusaSelection returns the saved [presets] selection. A missing or
// unreadable PrusaSlicer.ini simply means no selection is known.
func readPrusaSelection(file string) prusaSelection {
	data, err := readPrusaFile(file, maxPrusaFileBytes)
	if err != nil {
		return prusaSelection{}
	}
	ini, err := parsePrusaINI(data)
	if err != nil {
		return prusaSelection{}
	}
	presets := ini.sections["presets"]
	return prusaSelection{
		printer:  strings.TrimSpace(presets["printer"]),
		print:    strings.TrimSpace(presets["print"]),
		filament: strings.TrimSpace(presets["filament"]),
	}
}

// parsePrusaIndexedFile accepts real PrusaSlicer flat user presets only in
// the three trusted user preset folders. Vendor bundles, selections and
// sectioned INI files keep the original strict parser. Full printer, print
// and filament safety checks still decide whether any target is offered.
func parsePrusaIndexedFile(file string, user bool, data []byte) (*prusaINI, error) {
    parsed, originalError := parsePrusaINI(data)
    if originalError == nil || !user {
        return parsed, originalError
    }
    kind := filepath.Base(filepath.Dir(file))
    if kind != "printer" && kind != "print" && kind != "filament" {
        return nil, originalError
    }
    name := strings.TrimSuffix(filepath.Base(file), filepath.Ext(file))
    if name == "" || len(name) > 180 || strings.ContainsAny(name, "[]\r\n") || !utf8.Valid(data) {
        return nil, originalError
    }
    for _, line := range strings.Split(string(data), "\n") {
        if strings.HasPrefix(strings.TrimSpace(line), "[") {
            return nil, originalError // never reinterpret a malformed sectioned file
        }
    }
    header := "[" + kind + ":" + name + "]\n"
    if len(header)+len(data) > maxPrusaINIBytes {
        return nil, originalError
    }
    result, err := parsePrusaINI(append([]byte(header), data...))
    if err != nil {
        return nil, originalError
    }
    return result, nil
}

// addFile parses one profile file and indexes its presets. It returns the bytes
// it read so the caller can enforce the total budget.
func (index *prusaIndex) addFile(file string, user bool, limit int64) (int64, error) {
	data, err := readPrusaFile(file, limit)
	if err != nil {
		return 0, err
	}
	read := int64(len(data))
	ini, err := parsePrusaIndexedFile(file, user, data)
	if err != nil {
		return read, fmt.Errorf("profile file is malformed: %w", err)
	}
	for _, header := range ini.order {
		section := ini.sections[header]
		if header == "vendor" {
			if !user {
				index.vendors[file] = strings.TrimSpace(section["name"])
			}
			continue
		}
		kind, name, found := strings.Cut(header, ":")
		name = strings.TrimSpace(name)
		if !found || name == "" || (kind != "printer" && kind != "print" && kind != "filament") {
			continue
		}
		key := kind + ":" + name
		index.profiles[key] = append(index.profiles[key], prusaProfile{file: file, values: section})
		if user && kind == "printer" && !strings.HasPrefix(name, "*") {
			index.printers = append(index.printers, prusaPrinterSource{file: file, name: name})
		}
	}
	return read, nil
}

// finish builds the sorted, selectable name list for each kind. Names wrapped
// in asterisks are abstract parents and are never chosen as a preset.
func (index *prusaIndex) finish() {
	for key := range index.profiles {
		kind, name, _ := strings.Cut(key, ":")
		if strings.HasPrefix(name, "*") {
			continue
		}
		index.names[kind] = append(index.names[kind], name)
	}
	for kind := range index.names {
		sort.Strings(index.names[kind])
	}
}

// lookup finds one preset by kind and name. It prefers the definition in
// preferFile, then a single unambiguous definition. Two competing definitions
// fail closed.
func (index *prusaIndex) lookup(kind, name, preferFile string) (prusaProfile, error) {
	choices := index.profiles[kind+":"+name]
	if len(choices) == 0 {
		return prusaProfile{}, fmt.Errorf("%s %q is not installed", kind, name)
	}
	for _, choice := range choices {
		if choice.file == preferFile {
			return choice, nil
		}
	}
	if len(choices) == 1 {
		return choices[0], nil
	}
	return prusaProfile{}, fmt.Errorf("%s %q is defined in more than one file", kind, name)
}

// flatten merges a preset with its inherited parents. Parents apply in listed
// order, and the preset's own keys win. It also returns every file it used.
func (index *prusaIndex) flatten(kind, name, preferFile string, depth int) (prusaSection, []string, error) {
	if depth > maxPrusaInherit {
		return nil, nil, fmt.Errorf("inheritance is deeper than %d levels", maxPrusaInherit)
	}
	profile, err := index.lookup(kind, name, preferFile)
	if err != nil {
		return nil, nil, err
	}
	parents, err := splitPrusaList(profile.values["inherits"])
	if err != nil {
		return nil, nil, fmt.Errorf("%s %q has a malformed inherits list", kind, name)
	}
	merged := prusaSection{}
	origins := []string{profile.file}
	for _, parent := range parents {
		// A parent is looked up near the file that named it.
		values, parentOrigins, err := index.flatten(kind, parent, profile.file, depth+1)
		if err != nil {
			return nil, nil, err
		}
		for key, value := range values {
			merged[key] = value
		}
		origins = append(origins, parentOrigins...)
	}
	for key, value := range profile.values {
		merged[key] = value
	}
	return merged, origins, nil
}

func (index *prusaIndex) vendorName(origins []string) string {
	for _, file := range origins {
		if name := index.vendors[file]; name != "" {
			return name
		}
	}
	return prusaCustomVendor
}

// resolvePrinter applies every fail-closed check to one user printer preset.
func (index *prusaIndex) resolvePrinter(source prusaPrinterSource) (PrusaTarget, error) {
	printer, origins, err := index.flatten("printer", source.name, source.file, 0)
	if err != nil {
		return PrusaTarget{}, err
	}
	switch technology := strings.TrimSpace(printer["printer_technology"]); technology {
	case "FFF":
	case "":
		return PrusaTarget{}, fmt.Errorf("printer_technology is missing")
	default:
		return PrusaTarget{}, fmt.Errorf("printer technology %s is not supported", clipPrusaText(technology))
	}
	if strings.TrimSpace(printer["bed_shape"]) == "" {
		return PrusaTarget{}, fmt.Errorf("bed_shape is missing")
	}
	nozzle, err := prusaNozzle(printer["nozzle_diameter"])
	if err != nil {
		return PrusaTarget{}, err
	}
	// PrusaSlicer derives num_extruders from the nozzle list. Only single-tool
	// printers reach this point, so the value is always one.
	printer["num_extruders"] = "1"
	vendor := index.vendorName(origins)
	printName, err := index.pickPrint(source.name, printer, nozzle, vendor)
	if err != nil {
		return PrusaTarget{}, err
	}
	filamentName, err := index.pickFilament(source.name, printer, vendor)
	if err != nil {
		return PrusaTarget{}, err
	}
	model := strings.TrimSpace(printer["printer_model"])
	if model == "" {
		model = source.name
	}
	return PrusaTarget{
		ID:       prusaTargetID(source.name, source.file),
		Title:    source.name,
		Slicer:   prusaSlicerLabel,
		Vendor:   vendor,
		Model:    model,
		Nozzle:   nozzle,
		Print:    printName,
		Filament: filamentName,
	}, nil
}

// prusaChoice ranks one compatible preset. The preset the user last selected for
// this printer wins. Then presets from the printer's own vendor bundle beat
// presets from any other source. Within that group, a print is ranked by
// distance from half the nozzle diameter, and ties go to the name that sorts
// first, so the choice is deterministic.
type prusaChoice struct {
	name     string
	selected bool
	foreign  bool
	score    float64
}

func (c prusaChoice) beats(other prusaChoice) bool {
	if c.selected != other.selected {
		return c.selected
	}
	if c.foreign != other.foreign {
		return !c.foreign
	}
	if c.score != other.score {
		return c.score < other.score
	}
	return c.name < other.name
}

// pickPrint chooses the compatible print whose layer height is nearest half the
// nozzle, and that is no taller than 80 percent of it.
func (index *prusaIndex) pickPrint(printerName string, printer prusaSection, nozzle, vendor string) (string, error) {
	diameter, _ := strconv.ParseFloat(nozzle, 64)
	var best prusaChoice
	found := false
	for _, name := range index.names["print"] {
		values, origins, err := index.flatten("print", name, "", 0)
		if err != nil {
			continue
		}
		compatible, err := prusaCompatible(values, printerName, printer)
		if err != nil || !compatible {
			continue
		}
		height, err := strconv.ParseFloat(strings.TrimSpace(values["layer_height"]), 64)
		if err != nil || height <= 0 || height > diameter*0.8 {
			continue
		}
		choice := prusaChoice{
			name:     name,
			selected: index.isSelected("print", printerName, name),
			foreign:  index.vendorName(origins) != vendor,
			score:    math.Abs(height - diameter*0.5),
		}
		if !found || choice.beats(best) {
			best, found = choice, true
		}
	}
	if !found {
		return "", fmt.Errorf("no compatible print preset for this nozzle")
	}
	return best.name, nil
}

// isSelected reports whether PrusaSlicer.ini recorded this preset as the one in
// use for this printer. The selection is trusted only for the printer it names.
func (index *prusaIndex) isSelected(kind, printerName, name string) bool {
	if index.selected.printer != printerName {
		return false
	}
	if kind == "print" {
		return index.selected.print == name
	}
	return index.selected.filament == name
}

// pickFilament chooses the compatible PLA filament, preferring the printer's
// own vendor bundle and then name order.
func (index *prusaIndex) pickFilament(printerName string, printer prusaSection, vendor string) (string, error) {
	var best prusaChoice
	found := false
	for _, name := range index.names["filament"] {
		values, origins, err := index.flatten("filament", name, "", 0)
		if err != nil {
			continue
		}
		if strings.TrimSpace(values["filament_type"]) != "PLA" {
			continue
		}
		compatible, err := prusaCompatible(values, printerName, printer)
		if err != nil || !compatible {
			continue
		}
		choice := prusaChoice{
			name:     name,
			selected: index.isSelected("filament", printerName, name),
			foreign:  index.vendorName(origins) != vendor,
		}
		if !found || choice.beats(best) {
			best, found = choice, true
		}
	}
	if !found {
		return "", fmt.Errorf("no compatible PLA filament preset")
	}
	return best.name, nil
}

// prusaCompatible applies PrusaSlicer's precedence. An explicit list decides
// alone. Otherwise a condition decides. Otherwise every printer is compatible.
func prusaCompatible(preset prusaSection, printerName string, printer prusaSection) (bool, error) {
	listed, err := splitPrusaList(preset["compatible_printers"])
	if err != nil {
		return false, err
	}
	if len(listed) > 0 {
		for _, item := range listed {
			if item == printerName {
				return true, nil
			}
		}
		return false, nil
	}
	condition := strings.TrimSpace(preset["compatible_printers_condition"])
	if condition == "" {
		return true, nil
	}
	return evalPrusaCondition(condition, printer)
}

// prusaNozzle requires exactly one supported diameter. Multi-extruder printers
// are rejected until they have a validated conversion path.
func prusaNozzle(raw string) (string, error) {
	items, err := splitPrusaValues(raw)
	if err != nil {
		return "", fmt.Errorf("nozzle_diameter is malformed")
	}
	if len(items) != 1 {
		return "", fmt.Errorf("nozzle_diameter must list exactly one tool; multi-extruder printers are not supported")
	}
	value, err := strconv.ParseFloat(items[0], 64)
	if err != nil {
		return "", fmt.Errorf("nozzle_diameter is not numeric")
	}
	for _, supported := range prusaNozzles {
		reference, _ := strconv.ParseFloat(supported, 64)
		if math.Abs(reference-value) < 0.000001 {
			return supported, nil
		}
	}
	return "", fmt.Errorf("unsupported nozzle diameter %s mm", clipPrusaText(items[0]))
}

// prusaTargetID hashes the exact preset name and file, so IDs are stable for
// one install and unique across identical names in different folders.
func prusaTargetID(name, file string) string {
	sum := sha256.Sum256([]byte(file + "\x00" + name))
	return "prusa-" + orcaSlug(name, 40) + "-" + hex.EncodeToString(sum[:])[:10]
}

func clipPrusaText(text string) string {
	if len(text) > 32 {
		return text[:32]
	}
	return text
}

// listPrusaFiles returns regular .ini files in one folder. Symlinks, junctions
// and other special entries are skipped, and the count is bounded.
func listPrusaFiles(dir string, limit int) ([]string, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	files := []string{}
	for _, entry := range entries {
		if !entry.Type().IsRegular() || !strings.EqualFold(filepath.Ext(entry.Name()), ".ini") {
			continue
		}
		if len(files) >= limit {
			return nil, fmt.Errorf("more than %d profile files", limit)
		}
		files = append(files, filepath.Join(dir, entry.Name()))
	}
	return files, nil
}

// readPrusaFile reads one regular file no larger than limit. Error text never
// contains the path.
func readPrusaFile(file string, limit int64) ([]byte, error) {
	info, err := os.Lstat(file)
	if err != nil || !info.Mode().IsRegular() || info.Size() > limit {
		return nil, errPrusaUnreadable
	}
	handle, err := os.Open(file)
	if err != nil {
		return nil, errPrusaUnreadable
	}
	defer handle.Close()
	data, err := io.ReadAll(io.LimitReader(handle, limit+1))
	if err != nil || int64(len(data)) > limit {
		return nil, errPrusaUnreadable
	}
	return data, nil
}
