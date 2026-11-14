package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"

	"github.com/nocturney/slicebridge/native/presets"
	wailsruntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

type PrinterSettings struct {
	Nozzle    string            `json:"nozzle"`    // "auto" uses source matching; otherwise the physically installed nozzle
	Slicer    string            `json:"slicer"`    // "auto" or known local slicer installation
	Filaments map[string]string `json:"filaments"` // optional native preset per material type
}
type WindowSettings struct {
	Width  int `json:"width"`
	Height int `json:"height"`
}
type Settings struct {
	Version       int                        `json:"version"`
	Printers      map[string]PrinterSettings `json:"printers"`
	Window        WindowSettings             `json:"window"`
	DeveloperMode bool                       `json:"developerMode"`
}

type SettingsRecovery struct {
	Settings   Settings `json:"settings"`
	BackupPath string   `json:"backupPath,omitempty"`
}

func defaultSettings() Settings {
	m := map[string]PrinterSettings{}
	for t := range outputs {
		m[t] = PrinterSettings{Nozzle: "auto", Slicer: "auto", Filaments: map[string]string{}}
	}
	// New installs start with enough room for a readable source-file queue.
	// loadSettings still honors every already-saved user window dimension.
	return Settings{Version: 1, Printers: m, Window: WindowSettings{Width: 1280, Height: 900}}
}
func settingsPath() (string, error) {
	if v := os.Getenv("BRIDGE_SETTINGS_FILE"); v != "" {
		return v, nil
	}
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "Velvet Factory", "SliceBridge", "settings.json"), nil
}

// Windows antivirus and concurrent readers can briefly hold a non-shareable
// file handle while another goroutine atomically replaces settings.json.
// Retry only Windows sharing/lock violations; never treat arbitrary I/O,
// corrupt JSON or access-denied errors as valid settings.
func transientWindowsSettingsLock(err error) bool {
	return runtime.GOOS == "windows" &&
		(errors.Is(err, syscall.Errno(32)) || errors.Is(err, syscall.Errno(33)))
}

func readSettingsFile(path string) ([]byte, error) {
	data, err := os.ReadFile(path)
	for attempt := 0; attempt < 20 && transientWindowsSettingsLock(err); attempt++ {
		time.Sleep(5 * time.Millisecond)
		data, err = os.ReadFile(path)
	}
	return data, err
}

func renameSettingsFile(source, destination string) error {
	err := os.Rename(source, destination)
	for attempt := 0; attempt < 20 && transientWindowsSettingsLock(err); attempt++ {
		time.Sleep(5 * time.Millisecond)
		err = os.Rename(source, destination)
	}
	return err
}

func loadSettings() (Settings, error) {
	defaults := defaultSettings()
	file, err := settingsPath()
	if err != nil {
		return defaults, err
	}
	b, err := readSettingsFile(file)
	if errors.Is(err, os.ErrNotExist) {
		// Preserve existing nozzle, filament and window preferences when
		// upgrading from the original 3MF Bridge name. Never overwrite
		// the original: the new path is written on the next setting save.
		if os.Getenv("BRIDGE_SETTINGS_FILE") != "" {
			return defaults, nil
		}
		root, dirErr := os.UserConfigDir()
		if dirErr != nil {
			return defaults, dirErr
		}
		legacy := filepath.Join(root, "Velvet Factory", "3MF Bridge", "settings.json")
		b, err = readSettingsFile(legacy)
		if errors.Is(err, os.ErrNotExist) {
			return defaults, nil
		}
	}
	if err != nil {
		return defaults, err
	}
	if len(b) > 64*1024 {
		return defaults, fmt.Errorf("settings file is too large")
	}
	var current Settings
	if err = json.Unmarshal(b, &current); err != nil {
		return defaults, fmt.Errorf("invalid settings JSON: %w", err)
	}
	if current.Version != 1 {
		return defaults, fmt.Errorf("unsupported settings schema: %d", current.Version)
	}
	for target, pref := range current.Printers {
		if _, ok := outputs[target]; !ok {
			return defaults, fmt.Errorf("unknown saved printer: %s", target)
		}
		if err = validateSettings(target, pref, false); err != nil {
			return defaults, fmt.Errorf("saved %s: %w", target, err)
		}
		if pref.Nozzle == "" {
			pref.Nozzle = "auto"
		}
		if pref.Slicer == "" {
			pref.Slicer = "auto"
		}
		if pref.Filaments == nil {
			pref.Filaments = map[string]string{}
		}
		defaults.Printers[target] = pref
	}
	if current.Window.Width >= 720 && current.Window.Width <= 2000 {
		defaults.Window.Width = current.Window.Width
	}
	if current.Window.Height >= 500 && current.Window.Height <= 1400 {
		defaults.Window.Height = current.Window.Height
	}
	// Absent from older files, so it decodes to false and stays disabled.
	defaults.DeveloperMode = current.DeveloperMode
	return defaults, nil
}
func writeSettings(settings Settings) error {
	path, err := settingsPath()
	if err != nil {
		return err
	}
	dir := filepath.Dir(path)
	if err = os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(settings, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	file, err := os.CreateTemp(dir, ".settings-*.tmp")
	if err != nil {
		return err
	}
	name := file.Name()
	defer os.Remove(name)
	if err = file.Chmod(0600); err != nil {
		file.Close()
		return err
	}
	if _, err = file.Write(data); err != nil {
		file.Close()
		return err
	}
	if err = file.Sync(); err != nil {
		file.Close()
		return err
	}
	if err = file.Close(); err != nil {
		return err
	}
	if err = renameSettingsFile(name, path); err != nil {
		return err
	}
	return nil
}

// RecoverInvalidSettings is an opt-in recovery path for malformed or incompatible
// settings. It never edits an otherwise valid file and retains the exact bytes
// of the old file alongside the replacement. The UI must request confirmation.
func (a *App) RecoverInvalidSettings(confirmed bool) (SettingsRecovery, error) {
	if !confirmed {
		return SettingsRecovery{}, fmt.Errorf("settings recovery requires explicit confirmation")
	}
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()

	if valid, err := loadSettings(); err == nil {
		return SettingsRecovery{Settings: valid}, nil
	}
	path, err := settingsPath()
	if err != nil {
		return SettingsRecovery{}, err
	}
	info, err := os.Lstat(path)
	if err != nil {
		return SettingsRecovery{}, err
	}
	if !info.Mode().IsRegular() {
		return SettingsRecovery{}, fmt.Errorf("settings are not a regular file; recovery refused")
	}
	backup := path + ".invalid-" + time.Now().UTC().Format("20060102T150405.000000000") + ".bak"
	if err := os.Rename(path, backup); err != nil {
		return SettingsRecovery{}, fmt.Errorf("could not preserve original settings: %w", err)
	}
	fresh := defaultSettings()
	if err := writeSettings(fresh); err != nil {
		// If no replacement file was created, try to restore the original.
		if _, checkErr := os.Lstat(path); errors.Is(checkErr, os.ErrNotExist) {
			_ = os.Rename(backup, path)
		}
		return SettingsRecovery{}, fmt.Errorf("settings backup/recovery failed (%s): %w", backup, err)
	}
	return SettingsRecovery{Settings: fresh, BackupPath: backup}, nil
}

func validateSettings(target string, pref PrinterSettings, checkInstalled bool) error {
	spec, ok := presets.Specs[target]
	if !ok {
		return fmt.Errorf("invalid target printer")
	}
	if pref.Nozzle != "" && pref.Nozzle != "auto" {
		found := false
		for _, v := range spec.Nozzles {
			if v == pref.Nozzle {
				found = true
			}
		}
		if !found {
			return fmt.Errorf("nozzle %q is not supported by %s", pref.Nozzle, spec.Name)
		}
	}
	if !presets.IsValidSlicer(target, pref.Slicer) {
		return fmt.Errorf("invalid slicer selection %q", pref.Slicer)
	}
	if len(pref.Filaments) > 8 {
		return fmt.Errorf("too many saved filament types")
	}
	for material, name := range pref.Filaments {
		valid := material == "PLA" || material == "PETG" || material == "ABS" || material == "ASA" || material == "TPU"
		if !valid {
			return fmt.Errorf("invalid material type %q", material)
		}
		if len(name) > 200 || strings.ContainsAny(name, "\x00\r\n") {
			return fmt.Errorf("invalid saved %s preset name", material)
		}
	}
	if checkInstalled {
		// With an automatic nozzle, a source project can select 0.6 mm after
		// the user saved a vendor filament preset verified only for 0.4 mm.
		// Require an explicit physical nozzle instead of silently remapping.
		if pref.Nozzle == "" || pref.Nozzle == "auto" {
			for _, name := range pref.Filaments {
				if name != "" {
					return fmt.Errorf("choose a fixed physical nozzle before saving a filament preset; automatic nozzle follows the source project")
				}
			}
		}
		// A physical nozzle preference is valid without a slicer installed:
		// the user can configure their printer in advance. Conversion will
		// still fail closed if a required native vendor profile is missing.
		// Explicit slicer and filament overrides, however, must resolve now.
		if pref.Slicer != "" && pref.Slicer != "auto" {
			if _, _, _, err := presets.ResolveSlicer(target, pref.Slicer); err != nil {
				return err
			}
		}
		if len(pref.Filaments) > 0 {
			nozzle := pref.Nozzle
			if nozzle == "auto" || nozzle == "" {
				nozzle = "0.4"
			}
			if err := presets.ValidateOverrides(target, nozzle, pref.Slicer, pref.Filaments); err != nil {
				return err
			}
		}
	}
	return nil
}
func (a *App) GetSettings() (Settings, error) {
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	return loadSettings()
}

var errDeveloperModeOff = errors.New("Developer Mode is off: enable it in About before using browser extension setup")

// checkDeveloperMode fails closed: missing, corrupt or unreadable settings count
// as disabled. It does not take settingsMu, so the URI bridge can call it.
// writeSettings replaces the file atomically, so a read never sees a partial file.
func checkDeveloperMode() error {
	current, err := loadSettings()
	if err != nil {
		return fmt.Errorf("Developer Mode could not be verified: %w", err)
	}
	if !current.DeveloperMode {
		return errDeveloperModeOff
	}
	return nil
}

// requireDeveloperMode must be the first statement of every browser, OS
// registration or extension-file action, before any context or platform check.
func (a *App) requireDeveloperMode() error {
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	return checkDeveloperMode()
}

// GetDeveloperMode reports the saved local Developer Mode preference.
func (a *App) GetDeveloperMode() (bool, error) {
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	current, err := loadSettings()
	if err != nil {
		return false, err
	}
	return current.DeveloperMode, nil
}

// SetDeveloperMode persists the preference. A corrupt settings file is never
// overwritten: the load error is returned and nothing is written.
func (a *App) SetDeveloperMode(enabled bool) (bool, error) {
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	current, err := loadSettings()
	if err != nil {
		return false, err
	}
	current.DeveloperMode = enabled
	if err = writeSettings(current); err != nil {
		return false, err
	}
	return enabled, nil
}
func (a *App) SavePrinterSettings(target string, pref PrinterSettings) (Settings, error) {
	if err := validateSettings(target, pref, true); err != nil {
		return Settings{}, err
	}
	if pref.Nozzle == "" {
		pref.Nozzle = "auto"
	}
	if pref.Slicer == "" {
		pref.Slicer = "auto"
	}
	if pref.Filaments == nil {
		pref.Filaments = map[string]string{}
	}
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	current, err := loadSettings()
	if err != nil {
		return Settings{}, err
	}
	current.Printers[target] = pref
	if err = writeSettings(current); err != nil {
		return Settings{}, err
	}
	return current, nil
}

type ProfileChoices struct {
	Nozzles   []string               `json:"nozzles"`
	Slicers   []presets.SlicerChoice `json:"slicers"`
	Filaments []string               `json:"filaments"`
}

func (a *App) PrinterOptions(target, nozzle, slicer, material string) (ProfileChoices, error) {
	spec, ok := presets.Specs[target]
	if !ok {
		return ProfileChoices{}, fmt.Errorf("invalid target")
	}
	opts := ProfileChoices{Nozzles: spec.Nozzles, Slicers: presets.Slicers(target), Filaments: []string{}}
	if nozzle == "" || nozzle == "auto" {
		nozzle = "0.4"
	}
	if material == "" {
		material = "PLA"
	}
	if _, _, _, _, err := presets.Variant(target, nozzle); err != nil {
		return opts, err
	}
	list, err := presets.FilamentPresets(target, nozzle, slicer, material)
	if err == nil {
		opts.Filaments = list
	}
	return opts, err
}
func (a *App) RememberWindowSize() error {
	if a.ctx == nil {
		return nil
	}
	w, h := wailsruntime.WindowGetSize(a.ctx)
	if w < 720 || h < 500 || w > 2000 || h > 1400 {
		return nil
	}
	a.settingsMu.Lock()
	defer a.settingsMu.Unlock()
	current, err := loadSettings()
	if err != nil {
		return err
	}
	if current.Window.Width == w && current.Window.Height == h {
		return nil
	}
	current.Window = WindowSettings{w, h}
	return writeSettings(current)
}
