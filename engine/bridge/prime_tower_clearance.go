package bridge

import (
	"archive/zip"
	"encoding/json"
	"encoding/xml"
	"io"
	"math"
	"strconv"
	"strings"
)

// primeTowerProfileEnabled consults the resolved, in-memory target profile,
// never the original source printer's tower placement or a fabricated default.
func primeTowerProfileEnabled(settings map[string]any) bool {
	switch raw := settings["enable_prime_tower"].(type) {
	case string:
		return raw == "1" || strings.EqualFold(raw, "true")
	case bool:
		return raw
	case int:
		return raw == 1
	case float64:
		return raw == 1
	default:
		return false
	}
}

// towerPlatePositionAudit is a structural diagnostic, never a placement
// solution or clearance certification. Read the converted target project's
// native model_settings plate declarations first. Some valid Bambu/Orca 3MFs
// have none but already declare plates in slice_info.config: use those
// existing entries as the fallback, never invent or inject a new plate.
type towerPlatePositionAudit struct {
	plates, xEntries, yEntries int
}

func (a towerPlatePositionAudit) incomplete() bool {
	return a.plates > 0 && (a.xEntries < a.plates || a.yEntries < a.plates)
}

// status reports what the converted 3MF metadata permits us to CHECK, not
// whether any prime tower location is geometrically or physically safe.
// Keep the prior incomplete boolean for existing clients; unknown is a
// separate case rather than a false implication of successful verification.
func (a towerPlatePositionAudit) status(primeTowerReview bool) string {
	if !primeTowerReview {
		return "not_applicable"
	}
	if a.plates == 0 {
		return "plate_count_unknown"
	}
	if a.incomplete() {
		return "coordinates_incomplete"
	}
	return "coordinates_present_clearance_unverified"
}

// auditTargetTowerPlatePositions does not change a 3MF or vendor/user presets.
// A 2-plate Kobra3 3MF with zero or one coordinate pair was rejected by the
// installed OrcaSlicer 2.4.2 with exit 192, while a QA-only project with one
// pair per plate sliced. No coordinate or other slicer setting is inferred.
// For unsupported/malformed/non-Bambu metadata return unknown (zero plates),
// rather than preventing a previously valid editable project from converting.
// Read-only fallback to existing slice_info plates must not alter any 3MF bytes.
func auditTargetTowerPlatePositions(filename string) towerPlatePositionAudit {
	var result towerPlatePositionAudit
	zr, err := zip.OpenReader(filename)
	if err != nil {
		return result
	}
	defer zr.Close()
	const maxMetadata = 16 << 20
	var platesData, projectData, sliceInfoData []byte
	for _, file := range zr.File {
		if file.Name != "Metadata/model_settings.config" &&
			file.Name != "Metadata/project_settings.config" &&
			file.Name != "Metadata/slice_info.config" {
			continue
		}
		if file.UncompressedSize64 > maxMetadata {
			if file.Name == "Metadata/slice_info.config" {
				continue // optional fallback must not invalidate valid model_settings plates
			}
			return result
		}
		rc, err := file.Open()
		if err != nil {
			if file.Name == "Metadata/slice_info.config" {
				continue
			}
			return result
		}
		data, err := io.ReadAll(io.LimitReader(rc, maxMetadata+1))
		rc.Close()
		if err != nil || len(data) > maxMetadata {
			if file.Name == "Metadata/slice_info.config" {
				continue // keep authoritative native plate count if slice_info is bad
			}
			return result
		}
		switch file.Name {
		case "Metadata/model_settings.config":
			platesData = data
		case "Metadata/project_settings.config":
			projectData = data
		case "Metadata/slice_info.config":
			sliceInfoData = data
		}
	}
	if len(projectData) == 0 {
		return result
	}
	if len(platesData) > 0 {
		var modelSettings struct {
			XMLName xml.Name   `xml:"config"`
			Plates  []struct{} `xml:"plate"`
		}
		if err := xml.Unmarshal(platesData, &modelSettings); err != nil ||
			modelSettings.XMLName.Local != "config" {
			return result // malformed primary metadata must not be silently overridden
		}
		result.plates = len(modelSettings.Plates)
	}
	if result.plates == 0 && len(sliceInfoData) > 0 {
		var sliceInfo struct {
			XMLName xml.Name   `xml:"config"`
			Plates  []struct{} `xml:"plate"`
		}
		if err := xml.Unmarshal(sliceInfoData, &sliceInfo); err == nil &&
			sliceInfo.XMLName.Local == "config" {
			result.plates = len(sliceInfo.Plates)
		}
	}
	if result.plates == 0 {
		return result // no authoritative plate entry in either metadata part
	}
	var settings map[string]json.RawMessage
	if err := json.Unmarshal(projectData, &settings); err != nil {
		return towerPlatePositionAudit{}
	}
	result.xEntries = suppliedTowerPositionEntries(settings["wipe_tower_x"], result.plates)
	result.yEntries = suppliedTowerPositionEntries(settings["wipe_tower_y"], result.plates)
	return result
}

// Count only numeric, finite X/Y entries in positions that correspond to the
// declared plates. 0 is a supplied coordinate but NOT a clearance approval.
func suppliedTowerPositionEntries(raw json.RawMessage, plates int) int {
	if plates == 0 || len(raw) == 0 {
		return 0
	}
	var positions []json.RawMessage
	if err := json.Unmarshal(raw, &positions); err == nil {
		entries := 0
		for i := 0; i < len(positions) && i < plates; i++ {
			if numericTowerPosition(positions[i]) {
				entries++
			}
		}
		return entries
	}
	if numericTowerPosition(raw) {
		return 1
	}
	return 0
}

func numericTowerPosition(raw json.RawMessage) bool {
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return false
	}
	var n float64
	switch x := value.(type) {
	case float64:
		n = x
	case string:
		v, err := strconv.ParseFloat(strings.TrimSpace(x), 64)
		if err != nil {
			return false
		}
		n = v
	default:
		return false
	}
	return !math.IsNaN(n) && !math.IsInf(n, 0)
}
