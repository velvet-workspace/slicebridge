package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/nocturney/slicebridge/native/presets"
)

func writePNGCover(t *testing.T, root, rel string, shade color.RGBA) string {
	t.Helper()
	dest := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(dest), 0700); err != nil {
		t.Fatal(err)
	}
	picture := image.NewRGBA(image.Rect(0, 0, 128, 96))
	for y := 0; y < 96; y++ {
		for x := 0; x < 128; x++ {
			picture.Set(x, y, shade)
		}
	}
	var buffer bytes.Buffer
	if err := png.Encode(&buffer, picture); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dest, buffer.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
	return dest
}

func writeSampleCover(t *testing.T, root, rel string) string {
	t.Helper()
	return writePNGCover(t, root, rel, color.RGBA{25, 120, 90, 255})
}

// pngDataURIOf is computed from the file bytes directly, independently of the
// code under test, so a wrong file can never produce a matching expectation.
func pngDataURIOf(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return "data:image/png;base64," + base64.StdEncoding.EncodeToString(data)
}

// writeInstalledOrcaVendor lays out one complete Orca-family machine, using the
// layout presets reads: machine, process and filament folders under the vendor.
func writeInstalledOrcaVendor(t *testing.T, root, vendor, printerModel string) {
	t.Helper()
	machine := printerModel + " 0.4 nozzle"
	files := map[string]map[string]any{
		"machine/" + machine + ".json": {
			"name": machine, "printer_model": printerModel, "printer_technology": "FFF",
			"nozzle_diameter": []any{"0.4"}, "machine_start_gcode": "G28", "machine_end_gcode": "M104 S0",
			"default_print_profile": "0.20mm Test Standard", "default_filament_profile": []any{"Test PLA"},
		},
		"process/0.20mm Test Standard.json": {
			"name": "0.20mm Test Standard", "compatible_printers": []any{machine}, "layer_height": "0.2",
		},
		"filament/Test PLA.json": {
			"name": "Test PLA", "compatible_printers": []any{machine}, "filament_type": []any{"PLA"},
			"nozzle_temperature": []any{"210"}, "filament_flow_ratio": []any{"0.98"},
			"filament_max_volumetric_speed": []any{"18"},
		},
	}
	for rel, body := range files {
		data, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		path := filepath.Join(root, vendor, filepath.FromSlash(rel))
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
}

// useInstalledRoots supplies synthetic temp roots. Generic Orca discovery
// may still see real installed apps, so installedTargetIDFor explicitly
// selects only the synthetic custom-profile targets. Unused roots stay empty.
func useInstalledRoots(t *testing.T, u1, c5 string) {
	t.Helper()
	empty := t.TempDir()
	t.Setenv("BRIDGE_SLICER_PROFILE_U1", u1)
	t.Setenv("BRIDGE_SLICER_PROFILE_C5", c5)
	t.Setenv("BRIDGE_SLICER_PROFILE_CC2", empty)
	t.Setenv("BRIDGE_SLICER_PROFILE_QIDI", empty)
}

func installedTargetIDFor(t *testing.T, printerModel string) string {
	t.Helper()
	for _, target := range presets.DiscoverOrca().Targets {
		if target.Model == printerModel && target.Nozzle == "0.4" && target.Slicer == "Custom local profile" {
			return target.ID
		}
	}
	t.Fatalf("installed target for %q was not discovered", printerModel)
	return ""
}

func TestLocalCoverUsesInstalledSlicerOnly(t *testing.T) {
	root := t.TempDir()
	rel := "Snapmaker/Snapmaker U1_cover.png"
	writeSampleCover(t, root, rel)
	uri, ok := localPNGDataURI(root, rel)
	if !ok || !strings.HasPrefix(uri, "data:image/png;base64,") {
		t.Fatal("installed PNG rejected")
	}
	data, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(uri, "data:image/png;base64,"))
	if err != nil || !bytes.HasPrefix(data, []byte("\x89PNG\r\n\x1a\n")) {
		t.Fatalf("bad PNG data: %v", err)
	}
	model := CatalogPrinter{ID: "snapmaker-u1", LocalCoverCandidates: []string{rel}}
	art := findPrinterArtwork(model, []string{root})
	if !art.Found || art.Provenance != "local_installed_slicer" || art.DataURI != uri {
		t.Fatalf("artwork reply failed: %+v", art)
	}
	absent := findPrinterArtwork(model, []string{filepath.Join(root, "not-installed")})
	if absent.Found || absent.DataURI != "" {
		t.Fatal("must fall back when slicer is missing")
	}
}

func TestCoverPathSecurityAndNoArbitraryFileRead(t *testing.T) {
	for _, value := range []string{
		"../../secret_cover.png", "/etc/passwd_cover.png",
		"Vendor/../secret_cover.png", "Vendor\\machine_cover.png",
		"Vendor/machine.jpg", "Vendor/machine.svg",
		"C:/Windows_cover.png", "Vendor/evil\n_cover.png",
		"Vendor/foo/bar_cover.png",
	} {
		if validLocalCoverPath(value) {
			t.Fatalf("unsafe cover path accepted: %q", value)
		}
	}
	root := t.TempDir()
	file := writeSampleCover(t, root, "Qidi/Qidi X-Plus 5_cover.png")
	if _, ok := localPNGDataURI(root, "Qidi/Qidi X-Plus 5_cover.png"); !ok {
		t.Fatal("valid manufacturer image rejected")
	}
	// Non-image bytes renamed to _cover.png must never reach the browser.
	if err := os.WriteFile(file, []byte("<svg><script>alert(1)</script></svg>"), 0600); err != nil {
		t.Fatal(err)
	}
	if uri, ok := localPNGDataURI(root, "Qidi/Qidi X-Plus 5_cover.png"); ok || uri != "" {
		t.Fatal("untrusted SVG masquerading as PNG accepted")
	}
	// A symlink to a file outside the installed slicer root must be rejected.
	outside := t.TempDir()
	external := writeSampleCover(t, outside, "leak_cover.png")
	link := filepath.Join(root, "Qidi", "linked_cover.png")
	if err := os.Symlink(external, link); err == nil {
		if uri, ok := localPNGDataURI(root, "Qidi/linked_cover.png"); ok || uri != "" {
			t.Fatal("symlink escaped installed-slicer resource root")
		}
	}
}

func TestArtworkLookupModelIDNotPath(t *testing.T) {
	app := &App{}
	for _, query := range []string{"../../settings.json", "C:/Users/secret", "", "broken\\id"} {
		if _, err := app.PrinterArtwork(query); err == nil {
			t.Fatalf("untrusted filename used as model ID: %q", query)
		}
	}
	got, err := app.PrinterArtwork("definitely-not-real-printer")
	if err == nil || got.Found {
		t.Fatal("unknown printer must never open files")
	}
}

func TestPrinterCatalogueCoverReferencesAreLocalHintsNotBundledImages(t *testing.T) {
	catalog, err := catalogSnapshot()
	if err != nil {
		t.Fatal(err)
	}
	expected := map[string]string{
		"snapmaker-u1":             "Snapmaker/Snapmaker U1_cover.png",
		"flashforge-creator-5":     "Flashforge/Flashforge Creator 5_cover.png",
		"elegoo-centauri-carbon-2": "Elegoo/Elegoo Centauri Carbon 2_cover.png",
		"qidi-x-plus-5":            "X 5 Series/X-Plus 5_cover.png",
	}
	verified := 0
	for _, model := range catalog.Models {
		for _, ref := range model.LocalCoverCandidates {
			if !validLocalCoverPath(ref) {
				t.Fatalf("bad shipped path: %s", ref)
			}
		}
		if wanted, exists := expected[model.ID]; exists {
			for _, ref := range model.LocalCoverCandidates {
				if ref == wanted {
					verified++
					break
				}
			}
		}
		if model.ImagePath != nil || model.LogoPath != nil {
			t.Fatal("actual profile pictures may not be shipped")
		}
	}
	if verified != len(expected) {
		t.Fatalf("only %d/%d known local covers hinted", verified, len(expected))
	}
}

func TestInstalledTargetCoverUsesOwnInstallationBeforeOtherInstallations(t *testing.T) {
	own, other := t.TempDir(), t.TempDir()
	useInstalledRoots(t, own, other)
	writeInstalledOrcaVendor(t, own, "Acme", "Acme Mini")
	ownCover := writePNGCover(t, own, "Acme/Acme Mini_cover.png", color.RGBA{200, 30, 30, 255})
	writePNGCover(t, other, "Acme/Acme Mini_cover.png", color.RGBA{30, 30, 200, 255})

	reply, err := (&App{}).PrinterArtworkForInstalledTarget(installedTargetIDFor(t, "Acme Mini"))
	if err != nil {
		t.Fatal(err)
	}
	if !reply.Found || reply.Provenance != provenanceInstalledSlicer {
		t.Fatalf("exact installed cover not found: %+v", reply)
	}
	if reply.DataURI != pngDataURIOf(t, ownCover) {
		t.Fatal("cover must come from the target's own installation first")
	}
	// The payload is bytes and provenance only: no installation path leaks.
	payload, err := json.Marshal(reply)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(payload, []byte(own)) || bytes.Contains(payload, []byte(other)) {
		t.Fatal("installation path leaked into artwork reply")
	}
}

func TestInstalledTargetFallsBackOnlyToTheExactSameFileElsewhere(t *testing.T) {
	own, other := t.TempDir(), t.TempDir()
	useInstalledRoots(t, own, other)
	writeInstalledOrcaVendor(t, own, "Acme", "Acme Mini")
	// The target's own install has no cover, and a different model's cover sits
	// next to it. Only the exact file in another install may be used.
	writePNGCover(t, own, "Acme/Acme Mini Pro_cover.png", color.RGBA{1, 2, 3, 255})
	otherCover := writePNGCover(t, other, "Acme/Acme Mini_cover.png", color.RGBA{30, 30, 200, 255})

	reply, err := (&App{}).PrinterArtworkForInstalledTarget(installedTargetIDFor(t, "Acme Mini"))
	if err != nil {
		t.Fatal(err)
	}
	if !reply.Found || reply.DataURI != pngDataURIOf(t, otherCover) {
		t.Fatalf("exact same-model cover in another install not used: %+v", reply)
	}
}

func TestCreatorFiveCoverNeverStandsInForCreatorFivePro(t *testing.T) {
	root := t.TempDir()
	useInstalledRoots(t, root, t.TempDir())
	writeInstalledOrcaVendor(t, root, "Flashforge", "Flashforge Creator 5 Pro")
	writePNGCover(t, root, "Flashforge/Flashforge Creator 5_cover.png", color.RGBA{9, 9, 9, 255})

	// Scope this negative lookup to the temporary fixture. A real slicer
	// installation on the QA host may legitimately have an exact Pro cover,
	// which the production fallback is supposed to find.
	source := presets.OrcaArtworkSource{Root: root, Vendor: "Flashforge", Model: "Flashforge Creator 5 Pro"}
	reply := findInstalledTargetArtwork(source, nil)
	if reply.Found || reply.DataURI != "" {
		t.Fatalf("Creator 5 cover borrowed by Creator 5 Pro: %+v", reply)
	}

	proCover := writePNGCover(t, root, "Flashforge/Flashforge Creator 5 Pro_cover.png", color.RGBA{250, 1, 1, 255})
	reply = findInstalledTargetArtwork(source, nil)
	if !reply.Found || reply.DataURI != pngDataURIOf(t, proCover) {
		t.Fatalf("exact Creator 5 Pro cover not used: %+v", reply)
	}

	// Lookup is exact in both directions, whatever the installed folder holds.
	plain := presets.OrcaArtworkSource{Root: root, Vendor: "Flashforge", Model: "Flashforge Creator 5"}
	if found := findInstalledTargetArtwork(plain, nil); !found.Found {
		t.Fatal("plain Creator 5 exact cover not found")
	}
	pro := presets.OrcaArtworkSource{Root: t.TempDir(), Vendor: "Flashforge", Model: "Flashforge Creator 5 Pro Max"}
	if found := findInstalledTargetArtwork(pro, []string{root}); found.Found {
		t.Fatal("unlisted model borrowed a neighbouring model's cover")
	}
}

func TestInstalledTargetCoverRejectsSymlinkSvgAndUnsafeModelNames(t *testing.T) {
	root := t.TempDir()
	useInstalledRoots(t, root, t.TempDir())
	writeInstalledOrcaVendor(t, root, "Flashforge", "Flashforge Creator 5 Pro")
	// Negative PNG and symlink tests must not fall back to a real installed
	// exact-name cover; use the disposable vendor tree only.
	source := presets.OrcaArtworkSource{Root: root, Vendor: "Flashforge", Model: "Flashforge Creator 5 Pro"}

	// An SVG renamed to a cover name is not a PNG and must never be returned.
	svg := filepath.Join(root, "Flashforge", "Flashforge Creator 5 Pro_cover.png")
	if err := os.WriteFile(svg, []byte("<svg><script>alert(1)</script></svg>"), 0600); err != nil {
		t.Fatal(err)
	}
	if reply := findInstalledTargetArtwork(source, nil); reply.Found {
		t.Fatal("SVG disguised as cover accepted")
	}

	// A cover symlinked out of the installation must be refused.
	outside := t.TempDir()
	external := writeSampleCover(t, outside, "leak_cover.png")
	if err := os.Remove(svg); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(external, svg); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if reply := findInstalledTargetArtwork(source, nil); reply.Found || reply.DataURI != "" {
		t.Fatal("symlink escaped installed root")
	}

	// Model names that could steer the path are never turned into a file name.
	for _, model := range []string{"../../secret", "Flashforge/Creator", "", "C:Creator", "Creator\\5"} {
		source := presets.OrcaArtworkSource{Root: root, Vendor: "Flashforge", Model: model}
		if _, ok := installedTargetCoverPath(source); ok {
			t.Fatalf("unsafe printer_model became a cover path: %q", model)
		}
	}
	for _, vendor := range []string{"", "../Flashforge", "Flash/forge"} {
		source := presets.OrcaArtworkSource{Root: root, Vendor: vendor, Model: "Creator 5 Pro"}
		if _, ok := installedTargetCoverPath(source); ok {
			t.Fatalf("unsafe vendor became a cover path: %q", vendor)
		}
	}
}

func TestInstalledTargetLookupRejectsNonTargetIDsWithoutScanning(t *testing.T) {
	useInstalledRoots(t, t.TempDir(), t.TempDir())
	app := &App{}
	for _, id := range []string{
		"u1", "snapmaker-u1", "../../etc/passwd", "orca-../x", "",
		"C:/Users/secret", "orca-acme-mini", "ORCA-ACME-MINI-0123456789",
	} {
		if reply, err := app.PrinterArtworkForInstalledTarget(id); err == nil || reply.Found {
			t.Fatalf("invalid installed target accepted: %q", id)
		}
	}
	// A well-formed ID that no installed machine resolves to is an honest miss.
	if reply, err := app.PrinterArtworkForInstalledTarget("orca-acme-mini-0123456789"); err == nil || reply.Found {
		t.Fatalf("unresolvable installed target produced artwork: %+v", reply)
	}
}

func TestInstalledTargetWithoutCoverFallsBackHonestly(t *testing.T) {
	root := t.TempDir()
	useInstalledRoots(t, root, t.TempDir())
	writeInstalledOrcaVendor(t, root, "Acme", "Acme Mini")
	reply, err := (&App{}).PrinterArtworkForInstalledTarget(installedTargetIDFor(t, "Acme Mini"))
	if err != nil {
		t.Fatalf("a target with no cover should resolve without error: %v", err)
	}
	if reply.Found || reply.DataURI != "" || reply.Provenance != "" {
		t.Fatalf("missing cover must not invent artwork: %+v", reply)
	}
}

func TestInstalledOtherSlicerProvidesExactPro5ArtworkWithoutMachinePreset(t *testing.T) {
	owner := t.TempDir()
	bambu := t.TempDir() // Stand-in for another installed app's trusted resource root.
	source := presets.OrcaArtworkSource{Root: owner, Vendor: "Flashforge", Model: "Flashforge Creator 5 Pro"}
	writePNGCover(t, bambu, "Flashforge/Flashforge Creator 5_cover.png", color.RGBA{9, 0, 9, 255})
	if result := findInstalledTargetArtwork(source, []string{bambu}); result.Found {
		t.Fatal("plain Creator 5 image must not be used for Creator 5 Pro")
	}
	expected := writePNGCover(t, bambu, "Flashforge/Flashforge Creator 5 Pro_cover.png", color.RGBA{5, 8, 10, 255})
	result := findInstalledTargetArtwork(source, []string{bambu})
	if !result.Found || result.Provenance != provenanceInstalledSlicer ||
		result.DataURI != pngDataURIOf(t, expected) {
		t.Fatalf("a different locally installed slicer with exact same model art must be accepted: %+v", result)
	}
	own := writePNGCover(t, owner, "Flashforge/Flashforge Creator 5 Pro_cover.png", color.RGBA{240, 30, 20, 255})
	result = findInstalledTargetArtwork(source, []string{bambu})
	if !result.Found || result.DataURI != pngDataURIOf(t, own) {
		t.Fatal("own installed target cover must still take precedence over generic art")
	}
}

func TestInstalledCreatorFiveProConversionProducesModelNamedFile(t *testing.T) {
	root := t.TempDir()
	useInstalledRoots(t, t.TempDir(), root)
	writeInstalledOrcaVendor(t, root, "Flashforge", "Flashforge Creator 5 Pro")

	file := filepath.Join(t.TempDir(), "sample.3mf")
	data, err := os.ReadFile(loadFixture(t, "bambu-one-color.3mf"))
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(file, data, 0600); err != nil {
		t.Fatal(err)
	}
	target := installedTargetIDFor(t, "Flashforge Creator 5 Pro")
	result := configuredConvert(file, target, "0.4", PrinterSettings{Nozzle: "auto"})
	if !result.OK {
		t.Fatalf("installed Creator 5 Pro conversion did not succeed: %s %s", result.Code, result.Error)
	}
	expectedPrefix := presets.InstalledOrcaOutputPrefix(target, "Flashforge Creator 5 Pro", "Flashforge", "0.4")
	if result.Prefix != expectedPrefix || result.OutputName != expectedPrefix+"-sample.3mf" ||
		filepath.Base(result.OutputPath) != result.OutputName {
		t.Fatalf("target-specific naming was not applied: %+v", result)
	}
	if _, err := os.Stat(result.OutputPath); err != nil {
		t.Fatalf("actual 3MF output was not created: %v", err)
	}
	folderFiles, err := collectInputs([]string{filepath.Dir(file)})
	if err != nil || len(folderFiles) != 1 || folderFiles[0].Path != file {
		t.Fatalf("folder picker re-imported its own model-named output: %+v, %v", folderFiles, err)
	}
	manual, err := collectInputs([]string{result.OutputPath})
	if err != nil || len(manual) != 1 || manual[0].Path != result.OutputPath {
		t.Fatalf("explicitly reselecting output must remain possible: %+v, %v", manual, err)
	}
}
