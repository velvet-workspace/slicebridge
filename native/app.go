package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"sync"

	"github.com/nocturney/slicebridge/engine/bridge"
	"github.com/nocturney/slicebridge/native/presets"
	wailsruntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

const max3MFSize = 250 << 20

type App struct {
	ctx               context.Context
	mutex             sync.Mutex
	conversionSeq     uint64
	conversionCancels map[uint64]context.CancelFunc
	settingsMu        sync.Mutex
	lastFolder        string
	generated         map[string]string
	latestBatch       BatchReply
}

func (a *App) startup(ctx context.Context) { a.ctx = ctx }

// Called by the Wails frontend only AFTER the MakerWorld event listeners
// and first-run UI are ready. A startup URL may arrive in OnStartup or via
// macOS OnUrlOpen before JavaScript exists; this safely releases that queue.
func (a *App) MakerWorldImportEventsReady() { externalBridge.markFrontendReady() }

type FileInfo struct {
	Path      string `json:"path"`
	Name      string `json:"name"`
	Folder    string `json:"folder"`
	Size      int64  `json:"size"`
	SizeLabel string `json:"sizeLabel"`
}
type FileSelection struct {
	Canceled bool      `json:"canceled"`
	File     *FileInfo `json:"file,omitempty"`
}
type ConvertRequest struct {
	FilePath string   `json:"filePath"`
	Targets  []string `json:"targets"`
	Engine   string   `json:"engine,omitempty"`
}
type ConvertResult struct {
	OK         bool           `json:"ok"`
	Target     string         `json:"target"`
	Prefix     string         `json:"prefix"`
	OutputPath string         `json:"outputPath"`
	OutputName string         `json:"outputName"`
	Report     map[string]any `json:"report,omitempty"`
	Code       string         `json:"code,omitempty"`
	Error      string         `json:"error,omitempty"`
}
type ConvertReply struct {
	OK                  bool            `json:"ok"`
	Folder              string          `json:"folder"`
	SourceNozzle        string          `json:"sourceNozzle,omitempty"`
	SourceNozzleAssumed bool            `json:"sourceNozzleAssumed,omitempty"`
	Results             []ConvertResult `json:"results"`
	Failures            []ConvertResult `json:"failures"`
	Error               string          `json:"error,omitempty"`
}

var outputs = map[string]string{"u1": "U1", "c5": "Creator5", "cc2": "CentauriCarbon2", "qidi": "XPlus5"}

func fileInfo(path string) (*FileInfo, error) {
	if !strings.EqualFold(filepath.Ext(path), ".3mf") {
		return nil, fmt.Errorf("choose a .3mf file")
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	st, err := os.Stat(abs)
	if err != nil {
		return nil, err
	}
	if !st.Mode().IsRegular() || st.Size() == 0 || st.Size() > max3MFSize {
		return nil, fmt.Errorf("source must be a regular .3mf file up to 250 MB")
	}
	return &FileInfo{Path: abs, Name: filepath.Base(abs), Folder: filepath.Dir(abs), Size: st.Size(), SizeLabel: bytesLabel(st.Size())}, nil
}
func bytesLabel(size int64) string {
	if size < 1024 {
		return fmt.Sprintf("%d B", size)
	}
	if size < 1024*1024 {
		return fmt.Sprintf("%.1f KB", float64(size)/1024)
	}
	return fmt.Sprintf("%.1f MB", float64(size)/(1024*1024))
}
func (a *App) CheckFile(path string) (*FileInfo, error) { return fileInfo(path) }

func (a *App) SelectFile(lang string) (FileSelection, error) {
	title := "Select Bambu 3MF project"
	if lang == "he" {
		title = "בחירת פרויקט Bambu 3MF"
	}
	selected, err := wailsruntime.OpenFileDialog(a.ctx, wailsruntime.OpenDialogOptions{
		Title:   title,
		Filters: []wailsruntime.FileFilter{{DisplayName: "3MF projects (*.3mf)", Pattern: "*.3mf"}},
	})
	if err != nil {
		return FileSelection{}, err
	}
	if selected == "" {
		return FileSelection{Canceled: true}, nil
	}
	info, err := fileInfo(selected)
	if err != nil {
		return FileSelection{}, err
	}
	return FileSelection{File: info}, nil
}
func (a *App) Availability() []presets.Availability { return presets.Available() }

func tempVendorBundle(bundle *presets.Bundle) (string, func(), error) {
	dir, err := os.MkdirTemp("", "3mf-bridge-native-preset-")
	if err != nil {
		return "", nil, err
	}
	cleanup := func() { _ = os.RemoveAll(dir) }
	b, err := json.Marshal(bundle)
	if err != nil {
		cleanup()
		return "", nil, err
	}
	file := filepath.Join(dir, "vendor-profile.json")
	if err := os.WriteFile(file, b, 0600); err != nil {
		cleanup()
		return "", nil, err
	}
	return file, cleanup, nil
}
func convertTarget(source, target, sourceNozzle string) ConvertResult {
	return convertTargetWithContext(context.Background(), source, target, sourceNozzle, "")
}

func convertTargetWithContext(ctx context.Context, source, target, sourceNozzle, engine string) ConvertResult {
	result := ConvertResult{Target: target, Prefix: outputs[target]}
	dir := filepath.Dir(source)
	basename := filepath.Base(source)
	stem := strings.TrimSuffix(basename, filepath.Ext(basename))

	profileFile := ""
	if target != "u1" || sourceNozzle != "0.4" {
		bundle, err := presets.Load(target, sourceNozzle)
		if err != nil {
			result.Code = "TARGET_PROFILE_INVALID"
			result.Error = err.Error()
			return result
		}
		file, cleanup, err := tempVendorBundle(bundle)
		if err != nil {
			result.Code = "TARGET_PROFILE_INVALID"
			result.Error = err.Error()
			return result
		}
		defer cleanup()
		profileFile = file
	}
	for attempt := 1; attempt <= 100; attempt++ {
		filename := result.Prefix + "-" + basename
		if attempt > 1 {
			filename = fmt.Sprintf("%s-%s-%d%s", result.Prefix, stem, attempt, filepath.Ext(basename))
		}
		output := filepath.Join(dir, filename)
		_, err := os.Stat(output)
		if err == nil {
			continue
		}
		if !errors.Is(err, os.ErrNotExist) {
			result.Code = "OUTPUT_INVALID"
			result.Error = err.Error()
			return result
		}
		report, err := bridge.ConvertTargetContext(ctx, source, output, bridge.TargetRequest{
			Target: target, ProfileFile: profileFile, Engine: engine,
		})
		if err != nil {
			if bridge.ErrorCode(err) == "OUTPUT_EXISTS" {
				continue
			}
			result.Code = bridge.ErrorCode(err)
			result.Error = err.Error()
			return result
		}
		result.OK = true
		result.OutputName = filename
		result.OutputPath = output
		result.Report = report
		return result
	}
	result.Code = "OUTPUT_COLLISIONS"
	result.Error = "Too many outputs with the same name"
	return result
}
func (a *App) Convert(input ConvertRequest) ConvertReply {
	reply := ConvertReply{Results: []ConvertResult{}, Failures: []ConvertResult{}}
	engine, err := effectiveConversionEngine(input.Engine)
	if err != nil {
		reply.Error = err.Error()
		return reply
	}
	ctx, done := a.beginConversion(engine)
	defer done()
	info, err := fileInfo(input.FilePath)
	if err != nil {
		reply.Error = err.Error()
		return reply
	}
	reply.Folder = info.Folder
	if len(input.Targets) == 0 {
		reply.Error = "Select at least one target printer"
		return reply
	}
	seen := map[string]bool{}
	for _, target := range input.Targets {
		if _, ok := outputs[target]; !ok || seen[target] {
			reply.Error = "Invalid or duplicate target printer"
			return reply
		}
		seen[target] = true
	}
	inspection, err := bridge.InspectNozzle(info.Path)
	if err != nil {
		reply.Error = err.Error()
		return reply
	}
	sourceNozzle, ok := inspection["source_nozzle"].(string)
	if !ok {
		reply.Error = "Unknown nozzle diameter"
		return reply
	}
	reply.SourceNozzle = sourceNozzle
	reply.SourceNozzleAssumed, _ = inspection["source_nozzle_assumed"].(bool)
	results := make([]ConvertResult, len(input.Targets))
	var wg sync.WaitGroup
	for i, target := range input.Targets {
		wg.Add(1)
		go func(index int, printer string) {
			defer wg.Done()
			results[index] = convertTargetWithContext(ctx, info.Path, printer, sourceNozzle, engine)
		}(i, target)
	}
	wg.Wait()
	for _, result := range results {
		if result.OK {
			reply.Results = append(reply.Results, result)
		} else {
			reply.Failures = append(reply.Failures, result)
		}
	}
	reply.OK = len(reply.Results) > 0
	if !reply.OK {
		reply.Error = "All conversions failed"
	}
	if reply.OK {
		a.mutex.Lock()
		a.lastFolder = info.Folder
		a.mutex.Unlock()
	}
	return reply
}
func (a *App) OpenOutputFolder() error {
	a.mutex.Lock()
	folder := a.lastFolder
	a.mutex.Unlock()
	if folder == "" {
		return fmt.Errorf("no output folder yet")
	}
	if s, err := os.Stat(folder); err != nil || !s.IsDir() {
		return fmt.Errorf("output folder no longer exists")
	}
	var cmd *exec.Cmd
	if goruntime.GOOS == "windows" {
		cmd = exec.Command("explorer.exe", folder)
	} else if goruntime.GOOS == "darwin" {
		cmd = exec.Command("open", folder)
	} else {
		cmd = exec.Command("xdg-open", folder)
	}
	return cmd.Start()
}
func (a *App) FitWindow(width, height int) {
	if width < 640 {
		width = 640
	}
	if height < 460 {
		height = 460
	}
	if width > 1560 {
		width = 1560
	}
	if height > 1060 {
		height = 1060
	}
	wailsruntime.WindowSetSize(a.ctx, width, height)
	wailsruntime.WindowCenter(a.ctx)
}
