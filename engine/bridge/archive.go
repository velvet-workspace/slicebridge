package bridge

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"path"
	"regexp"
	"sort"
	"strings"
)

const maxCompressedBytes int64 = 250 << 20
const maxUncompressedBytes uint64 = 1024 << 20

var cacheName = regexp.MustCompile("^Metadata/plate_[0-9]+\\.(gcode(\\.md5)?|json)$")
var cacheReference = regexp.MustCompile(`(?i)plate_[0-9]+\.(?:gcode(?:\.md5)?|json)`)

type packageFacts struct {
	vertices     int
	triangles    int
	sliceCaches  int
	printerModel string
}

func isCache(s string) bool {
	s = strings.TrimPrefix(strings.ReplaceAll(s, "\\", "/"), "/")
	return cacheName.MatchString(s)
}

func inspectPackage(filename string) (packageFacts, error) {
	var facts packageFacts
	st, err := os.Stat(filename)
	if err != nil {
		return facts, err
	}
	if !st.Mode().IsRegular() || st.Size() == 0 || st.Size() > maxCompressedBytes {
		return facts, fmt.Errorf("3MF must be a nonempty regular file up to 250 MB")
	}
	zr, err := zip.OpenReader(filename)
	if err != nil {
		return facts, err
	}
	defer zr.Close()
	if len(zr.File) > 10000 {
		return facts, fmt.Errorf("archive contains too many entries")
	}
	var total uint64
	hasModel, hasContentTypes := false, false
	seen := make(map[string]bool)
	for _, f := range zr.File {
		entryName := strings.TrimSuffix(f.Name, "/")
		cleaned := path.Clean(entryName)
		if cleaned != entryName || strings.HasPrefix(cleaned, "../") || strings.HasPrefix(cleaned, "/") || strings.Contains(f.Name, "\\") {
			return facts, fmt.Errorf("unsafe archive entry %q", f.Name)
		}
		if seen[f.Name] {
			return facts, fmt.Errorf("duplicate archive entry %q", f.Name)
		}
		seen[f.Name] = true
		if f.UncompressedSize64 > maxUncompressedBytes || total > maxUncompressedBytes-f.UncompressedSize64 {
			return facts, fmt.Errorf("archive exceeds decompressed size limit")
		}
		total += f.UncompressedSize64
		if f.Name == "[Content_Types].xml" {
			hasContentTypes = true
		}
		if isCache(f.Name) {
			facts.sliceCaches++
		}
		if strings.HasPrefix(f.Name, "3D/") && strings.HasSuffix(strings.ToLower(f.Name), ".model") {
			hasModel = true
			rc, err := f.Open()
			if err != nil {
				return facts, err
			}
			dec := xml.NewDecoder(io.LimitReader(rc, 200<<20))
			for {
				tok, err := dec.Token()
				if err == io.EOF {
					break
				}
				if err != nil {
					rc.Close()
					return facts, fmt.Errorf("invalid 3D XML: %w", err)
				}
				if start, ok := tok.(xml.StartElement); ok {
					switch start.Name.Local {
					case "vertex":
						facts.vertices++
					case "triangle":
						facts.triangles++
					}
				}
			}
			rc.Close()
		}
		if f.Name == "Metadata/project_settings.config" {
			rc, err := f.Open()
			if err != nil {
				return facts, err
			}
			b, err := io.ReadAll(io.LimitReader(rc, 8<<20))
			rc.Close()
			if err != nil {
				return facts, err
			}
			var settings map[string]any
			if err := json.Unmarshal(b, &settings); err != nil {
				return facts, fmt.Errorf("project settings JSON: %w", err)
			}
			facts.printerModel, _ = settings["printer_model"].(string)
			if facts.printerModel == "" {
				facts.printerModel, _ = settings["printer_settings_id"].(string)
			}
		}
	}
	if !hasModel || facts.triangles == 0 || facts.vertices == 0 {
		return facts, fmt.Errorf("editable 3D mesh geometry is required (sliced-only 3MF is unsupported)")
	}
	if !hasContentTypes {
		return facts, fmt.Errorf("missing [Content_Types].xml")
	}
	return facts, nil
}

func stripSliceCache(source, dest string, preserved []string) (int, error) {
	return stripSliceCacheForQIDI(source, dest, preserved, false)
}

// stripSliceCacheForQIDI carries the same cache removal and atomic output
// semantics; only QIDI's final 3D model gets the tested importer metadata.
func stripSliceCacheForQIDI(source, dest string, preserved []string, qidi bool) (int, error) {
	zr, err := zip.OpenReader(source)
	if err != nil {
		return 0, err
	}
	defer zr.Close()
	out, err := os.Create(dest)
	if err != nil {
		return 0, err
	}
	defer out.Close()
	zw := zip.NewWriter(out)
	removed := 0
	qidiModelAdded := false
	for _, f := range zr.File {
		if isCache(f.Name) {
			removed++
			continue
		}
		if qidi && f.Name == "3D/3dmodel.model" {
			if qidiModelAdded {
				return removed, fmt.Errorf("duplicate primary QIDI 3MF model part")
			}
			if f.UncompressedSize64 > 200<<20 {
				return removed, fmt.Errorf("QIDI model XML exceeds 200 MiB")
			}
			rc, err := f.Open()
			if err != nil {
				return removed, err
			}
			data, err := io.ReadAll(io.LimitReader(rc, 200<<20+1))
			rc.Close()
			if err != nil {
				return removed, err
			}
			if len(data) > 200<<20 {
				return removed, fmt.Errorf("QIDI model XML exceeds 200 MiB")
			}
			edited, err := setQIDIApplicationMetadata(data)
			if err != nil {
				return removed, err
			}
			header := f.FileHeader
			w, err := zw.CreateHeader(&header)
			if err != nil {
				return removed, err
			}
			if _, err := w.Write(edited); err != nil {
				return removed, err
			}
			qidiModelAdded = true
			continue
		}
		if f.Name == "Metadata/project_settings.config" && len(preserved) > 0 {
			rc, err := f.Open()
			if err != nil {
				return removed, err
			}
			data, err := io.ReadAll(io.LimitReader(rc, 8<<20))
			rc.Close()
			if err != nil {
				return removed, err
			}
			edited, err := markProcessOverrides(data, preserved)
			if err != nil {
				return removed, err
			}
			w, err := zw.Create(f.Name)
			if err != nil {
				return removed, err
			}
			if _, err := w.Write(edited); err != nil {
				return removed, err
			}
			continue
		}
		if f.Name == "[Content_Types].xml" || strings.HasSuffix(f.Name, ".rels") {
			rc, err := f.Open()
			if err != nil {
				return removed, err
			}
			data, err := io.ReadAll(io.LimitReader(rc, 8<<20))
			rc.Close()
			if err != nil {
				return removed, err
			}
			if cacheReference.Match(data) {
				edited, err := stripCacheReferences(data)
				if err != nil {
					return removed, fmt.Errorf("cannot sanitize %s: %w", f.Name, err)
				}
				w, err := zw.Create(f.Name)
				if err != nil {
					return removed, err
				}
				if _, err := w.Write(edited); err != nil {
					return removed, err
				}
				continue
			}
		}
		writer, err := zw.CreateRaw(&f.FileHeader)
		if err != nil {
			return removed, err
		}
		raw, err := f.OpenRaw()
		if err != nil {
			return removed, err
		}
		if _, err := io.Copy(writer, raw); err != nil {
			return removed, err
		}
	}
	if qidi && !qidiModelAdded {
		return removed, fmt.Errorf("QIDI conversion is missing primary 3D/3dmodel.model")
	}
	if err := zw.Close(); err != nil {
		return removed, err
	}
	if err := out.Sync(); err != nil {
		return removed, err
	}
	return removed, nil
}

func stripCacheReferences(data []byte) ([]byte, error) {
	dec := xml.NewDecoder(bytes.NewReader(data))
	var buf bytes.Buffer
	enc := xml.NewEncoder(&buf)
	skip := 0
	for {
		token, err := dec.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		if skip > 0 {
			switch token.(type) {
			case xml.StartElement:
				skip++
			case xml.EndElement:
				skip--
			}
			continue
		}
		if st, ok := token.(xml.StartElement); ok {
			if st.Name.Local == "Override" || st.Name.Local == "Relationship" {
				discard := false
				for _, a := range st.Attr {
					if a.Name.Local == "PartName" || a.Name.Local == "Target" {
						s := strings.ReplaceAll(a.Value, "\\", "/")
						s = strings.TrimPrefix(strings.TrimPrefix(s, "../"), "/")
						if isCache(s) || isCache("Metadata/"+path.Base(s)) {
							discard = true
						}
					}
				}
				if discard {
					skip = 1
					continue
				}
			}
		}
		// encoding/xml inserts the default namespace automatically. Keeping an
		// identical explicit xmlns attribute would generate duplicate xmlns and
		// make genuine Bambu/MakerWorld project relationships invalid XML.
		if st, ok := token.(xml.StartElement); ok && st.Name.Space != "" {
			attrs := st.Attr[:0]
			for _, a := range st.Attr {
				if a.Name.Space == "" && a.Name.Local == "xmlns" && a.Value == st.Name.Space {
					continue
				}
				attrs = append(attrs, a)
			}
			st.Attr = attrs
			token = st
		}
		if err := enc.EncodeToken(token); err != nil {
			return nil, err
		}
	}
	if err := enc.Flush(); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func markProcessOverrides(raw []byte, preserved []string) ([]byte, error) {
	var cfg map[string]any
	if err := json.Unmarshal(raw, &cfg); err != nil {
		return nil, err
	}
	slots := 4
	if fs, ok := cfg["filament_colour"].([]any); ok && len(fs) > 0 {
		slots = len(fs)
	}
	diff, _ := cfg["different_settings_to_system"].([]any)
	if len(diff) < slots+2 {
		expanded := make([]any, slots+2)
		copy(expanded, diff)
		diff = expanded
	}
	marked := make(map[string]bool)
	if prior, ok := diff[0].(string); ok {
		for _, key := range strings.Split(prior, ";") {
			if strings.TrimSpace(key) != "" {
				marked[key] = true
			}
		}
	}
	for _, key := range preserved {
		marked[key] = true
	}
	names := make([]string, 0, len(marked))
	for key := range marked {
		names = append(names, key)
	}
	sort.Strings(names)
	diff[0] = strings.Join(names, ";")
	for i := 1; i < len(diff); i++ {
		if diff[i] == nil {
			diff[i] = ""
		}
	}
	cfg["different_settings_to_system"] = diff
	return json.MarshalIndent(cfg, "", "  ")
}
