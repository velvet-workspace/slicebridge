package bridge

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// Compare the bounded Rust slice-info XML stage against the *actual* Go
// converter's slice_info.config part. The 6->4 case aggregates two source
// filament IDs into shared target slots instead of silently losing usage.
func TestRustSliceInfoConverted3MFParity(t *testing.T) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		t.Skip("set SLICEBRIDGE_RUST_PROBE to a compiled Rust probe")
	}
	cases := []struct {
		source int
		target int
	}{
		{1, 4},
		{4, 4},
		{6, 6},
		{6, 4},
	}
	for _, tc := range cases {
		t.Run(fmt.Sprintf("%d_to_%d", tc.source, tc.target), func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "source.3mf")
			goOutput := filepath.Join(dir, "go.3mf")
			fixture(t, source, tc.source)
			before, err := os.ReadFile(source)
			if err != nil {
				t.Fatal(err)
			}
			input := inspectZipEntries(t, source)
			sourceXML := string(input["Metadata/slice_info.config"])
			if sourceXML == "" {
				t.Fatal("fixture missing slice_info.config")
			}
			info, err := converter.Inspect(source)
			if err != nil {
				t.Fatal(err)
			}
			slots, mapping, err := sourceSlotPlan(info.Filaments)
			if err != nil {
				t.Fatal(err)
			}
			if tc.target < len(slots) {
				slots = slots[:tc.target]
				for sourceID := range mapping {
					if mapping[sourceID] > tc.target {
						mapping[sourceID] = (sourceID-1)%tc.target + 1
					}
				}
			}
			profile := printer.Builtin("snapmaker-u1")
			if profile == nil {
				t.Fatal("missing builtin test profile")
			}
			profile.FilamentSlots = tc.target
			if _, err := converter.Convert(source, goOutput, converter.Options{
				Printer: profile, Slots: slots, Mapping: mapping,
				Supports: converter.SupportsOff,
			}); err != nil {
				t.Fatalf("real Go conversion declined fixture: %v", err)
			}
			// Go's conversionPlan uses padded slots, including empty hardware
			// bays represented as white PLA.
			padded := make([]converter.Slot, tc.target)
			for i := range padded {
				if i < len(slots) {
					padded[i] = slots[i]
				} else {
					padded[i] = converter.Slot{Color: "#FFFFFF", Type: "PLA"}
				}
			}
			reqMap := map[string]int{}
			for sourceID, target := range mapping {
				reqMap[strconv.Itoa(sourceID)] = target
			}
			reply, rustErr := runRustMaterialCommand(t, rust, "--rewrite-slice-info", map[string]any{
				"slice_info_xml": sourceXML,
				"model_id":       profile.ModelID,
				"mapping":        reqMap,
				"slots":          padded,
			})
			if rustErr != nil || reply["ok"] != true {
				t.Fatalf("Rust rejected Go-valid slice_info: %v %v", rustErr, reply)
			}
			xml, ok := reply["slice_info_xml"].(string)
			if !ok {
				t.Fatalf("Rust did not return slice_info XML: %v", reply)
			}
			goPart := inspectZipEntries(t, goOutput)["Metadata/slice_info.config"]
			left, right := semanticXMLTokens(t, goPart), semanticXMLTokens(t, []byte(xml))
			if !reflect.DeepEqual(left, right) {
				// Structured diagnostic rather than raw ZIP byte comparison:
				// Go and Rust may serialize equivalent XML differently.
				t.Fatalf("Go/Rust slice_info semantics differ\nGo XML: %s\nRust XML: %s", goPart, xml)
			}
			now, err := os.ReadFile(source)
			if err != nil || !bytes.Equal(before, now) {
				t.Fatal("source fixture unexpectedly modified")
			}
		})
	}
}
