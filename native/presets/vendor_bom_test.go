package presets

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func vendorBOM(data []byte) []byte {
	return append([]byte{0xEF, 0xBB, 0xBF}, data...)
}

// The installed Snapmaker Orca 2.4.1 tree contains exactly two legitimate
// UTF-8 BOM files in its process/inheritance chain. Regression fixtures use
// isolated synthetic files, never the user's installed OEM or user presets.
func TestVendorCatalogAcceptsUTF8BOMInInheritedProcess(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "Snapmaker", "process")
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	for name, v := range map[string]map[string]any{
		"parent": {
			"name": "parent", "type": "process",
			"layer_height": "0.2", "wall_loops": "3",
		},
		"0.20mm Standard @Snapmaker U1 (0.4 nozzle)": {
			"name":     "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
			"inherits": "parent", "type": "process", "wall_loops": "4",
		},
	} {
		raw, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, name+".json"), vendorBOM(raw), 0600); err != nil {
			t.Fatal(err)
		}
	}
	ix, err := readCatalog(root, Spec{Vendor: "Snapmaker"})
	if err != nil {
		t.Fatalf("BOM-equipped vendor catalog refused: %v", err)
	}
	flattened, err := flatten(ix, "process", "0.20mm Standard @Snapmaker U1 (0.4 nozzle)", map[string]bool{})
	if err != nil {
		t.Fatal(err)
	}
	if flattened["wall_loops"] != "4" || flattened["layer_height"] != "0.2" {
		t.Fatalf("lost inherited/overriding process values: %+v", flattened)
	}
	if _, exists := flattened["inherits"]; exists {
		t.Fatal("flatten leaked inherits field")
	}
}

func TestOrcaDiscoveryAcceptsUTF8BOMInMachineAndProcess(t *testing.T) {
	root := writeOrcaTree(t, "Acme", acmeSample())
	for _, rel := range []string{
		acmeMachine,
		acmeProcess,
	} {
		file := filepath.Join(root, "Acme", filepath.FromSlash(rel))
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(file, vendorBOM(data), 0600); err != nil {
			t.Fatal(err)
		}
	}
	disc := discoverFrom(root)
	if len(disc.Targets) != 1 || len(disc.Rejected) != 0 {
		t.Fatalf("native Orca discovery must accept actual UTF8 BOM files: %+v", disc)
	}
	b, err := resolveOrca([]slicerLocation{{id: "test", name: "Test Orca", root: root}}, disc.Targets[0].ID)
	if err != nil {
		t.Fatal(err)
	}
	if b.Profile.ModelID != "Acme Mini" || b.Profile.ProjectSettings["machine_start_gcode"] != "G28" {
		t.Fatalf("BOM profile resolution changed machine authority: %+v", b.Profile)
	}
}

func TestVendorPresetBOMDecoderOnlyAcceptsExactLeadingSignature(t *testing.T) {
	for name, test := range map[string]struct {
		raw      []byte
		accepted bool
	}{
		"plain":                  {[]byte(`{"name":"machine"}`), true},
		"utf8_bom":               {vendorBOM([]byte(`{"name":"machine"}`)), true},
		"bom_not_at_start":       {append([]byte(" "), vendorBOM([]byte(`{"name":"machine"}`))...), false},
		"partial_bom":            {append([]byte{0xEF, 0xBB}, []byte(`{"name":"machine"}`)...), false},
		"utf16":                  {[]byte{0xFF, 0xFE, 0x7B, 0x00}, false},
		"invalid_json_after_bom": {vendorBOM([]byte("{not-json}")), false},
	} {
		t.Run(name, func(t *testing.T) {
			var parsed map[string]any
			err := unmarshalVendorPresetJSON(test.raw, &parsed)
			if (err == nil) != test.accepted {
				t.Fatalf("acceptance mismatch expected %v, error %v", test.accepted, err)
			}
			if test.accepted && parsed["name"] != "machine" {
				t.Fatalf("parsed field mismatch: %v", parsed)
			}
		})
	}
	var parsed map[string]any
	if err := unmarshalVendorPresetJSON(bytes.Repeat([]byte{0xEF, 0xBB, 0xBF}, 2), &parsed); err == nil {
		t.Fatal("double BOM must be refused")
	}
}
