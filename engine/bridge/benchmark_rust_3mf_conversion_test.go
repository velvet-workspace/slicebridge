package bridge

import (
	"archive/zip"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/BryanSmee/bl_to_std/pkg/converter"
	"github.com/BryanSmee/bl_to_std/pkg/printer"
)

// BenchmarkRustGo3MFArchive measures the real subprocess Rust CLI, including
// startup and cache sanitization, versus the in-process Go upstream converter
// plus the production Go cache sanitizer. These are different integration
// boundaries; do not misinterpret this as pure language/runtime speed.
// Inputs are tiny test fixtures, never representative production 3MF sizes.
func BenchmarkRustGo3MFArchive(b *testing.B) {
	rust := os.Getenv("SLICEBRIDGE_RUST_PROBE")
	if rust == "" {
		b.Skip("set SLICEBRIDGE_RUST_PROBE")
	}

	for _, fixtureName := range []string{
		"bambu-one-color.3mf",
		"bambu-five-colors.3mf",
	} {
		b.Run(fixtureName, func(b *testing.B) {
			src := filepath.Join("..", "..", "tests", "fixtures", fixtureName)
			st, err := os.Stat(src)
			if err != nil {
				b.Fatal(err)
			}
			archive, err := zip.OpenReader(src)
			if err != nil {
				b.Fatal(err)
			}
			var sourceSettings map[string]any
			for _, f := range archive.File {
				if f.Name != "Metadata/project_settings.config" {
					continue
				}
				r, err := f.Open()
				if err != nil {
					archive.Close()
					b.Fatal(err)
				}
				data, err := io.ReadAll(io.LimitReader(r, 8<<20))
				r.Close()
				if err != nil {
					archive.Close()
					b.Fatal(err)
				}
				if err := json.Unmarshal(data, &sourceSettings); err != nil {
					archive.Close()
					b.Fatal(err)
				}
			}
			archive.Close()
			if sourceSettings == nil {
				b.Fatal("source settings absent")
			}
			inspection, err := converter.Inspect(src)
			if err != nil {
				b.Fatal(err)
			}
			slots, mapping, err := sourceSlotPlan(inspection.Filaments)
			if err != nil {
				b.Fatal(err)
			}
			profile := printer.Builtin("snapmaker-u1")
			if profile == nil {
				b.Fatal("missing test profile")
			}
			if len(slots) > profile.FilamentSlots {
				profile.FilamentSlots = len(slots)
			}
			mappingJSON := make(map[string]int, len(mapping))
			for from, to := range mapping {
				mappingJSON[strconv.Itoa(from)] = to
			}
			plan := map[string]any{
				"profile":         profile,
				"slots":           slots,
				"slot_count":      profile.FilamentSlots,
				"supports":        false,
				"source_settings": sourceSettings,
				"mapping":         mappingJSON,
				"preserved":       []string{},
			}
			data, err := json.Marshal(plan)
			if err != nil {
				b.Fatal(err)
			}
			temp := b.TempDir()
			planPath := filepath.Join(temp, "plan.json")
			if err := os.WriteFile(planPath, data, 0600); err != nil {
				b.Fatal(err)
			}
			// Fixed output names are removed between iterations. Only new
			// scratch paths are ever opened; installed apps are never touched.
			goRaw := filepath.Join(temp, "upstream.3mf")
			goClean := filepath.Join(temp, "go-clean.3mf")
			rustFinal := filepath.Join(temp, "rust-clean.3mf")

			b.Run("GoLibraryAndSanitize", func(b *testing.B) {
				b.SetBytes(st.Size())
				b.ReportAllocs()
				for n := 0; n < b.N; n++ {
					_, err := converter.Convert(src, goRaw, converter.Options{
						Printer: profile, Slots: slots, Mapping: mapping,
						Supports: converter.SupportsOff,
					})
					if err != nil {
						b.Fatal(err)
					}
					if _, err := stripSliceCache(goRaw, goClean, nil); err != nil {
						b.Fatal(err)
					}
					if err := os.Remove(goRaw); err != nil {
						b.Fatal(err)
					}
					if err := os.Remove(goClean); err != nil {
						b.Fatal(err)
					}
				}
			})
			b.Run("RustSubprocessAndSanitize", func(b *testing.B) {
				b.SetBytes(st.Size())
				for n := 0; n < b.N; n++ {
					out, err := exec.Command(rust,
						"--convert-archive", planPath, src, rustFinal).Output()
					if err != nil {
						b.Fatalf("Rust process failed: %v", err)
					}
					var reply struct {
						OK bool `json:"ok"`
					}
					if err := json.Unmarshal(out, &reply); err != nil || !reply.OK {
						b.Fatalf("Rust output not successful: %s, %v", out, err)
					}
					if err := os.Remove(rustFinal); err != nil {
						b.Fatal(err)
					}
				}
			})
			b.Logf("benchmark is CPU/I-O/process overhead for a tiny %s (%d bytes); NOT a printer speed test", fmt.Sprintf("%s", fixtureName), st.Size())
		})
	}
}
