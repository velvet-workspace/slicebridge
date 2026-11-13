# SliceBridge: experimental Rust 3MF components

**Development only. Not integrated with the installed SliceBridge application.**

This small Rust crate incrementally ports bounded 3MF archive, source-slot
and material-settings operations from the existing Go bridge engine. The
production conversion engine and the Go/Wails desktop app remain unchanged.

## Implemented

1. **Read-only 3MF inspection** (default command). Reports the number of
   geometry vertices and triangles, cached slice entries and source printer
   label. It rejects unsafe ZIP paths, duplicate names, missing editable
   geometry, malformed XML and archive size/entry-limit violations.
2. **Optional stale-slice cleanup** (`--strip-cache`). Creates a *new*
   3MF file, removing `Metadata/plate_*.gcode`, `*.gcode.md5`, and
   `plate_*.json` entries and their stale OPC XML references while
   preserving 3D models, thumbnails and other ZIP contents. Source never
   changes. Output creation is atomic and fails if the destination exists.
3. **Source filament slot planning** (`--plan-slots request.json`) preserves
   all source IDs and keeps matching colors in distinct virtual channels.
   Invalid IDs, duplicate IDs, missing material/color and palettes beyond
   64 slots are rejected. Support filaments are flagged, not merged.
4. **Installed native material-parameter mapping** (`--map-materials request.json`)
   accepts an in-memory snapshot of a Go target bundle plus source filaments
   and returns mapped project settings as JSON. It refuses missing or unknown
   installed material presets and partial safety-critical temperature/flow
   settings. Hardware-owned nozzle and acceleration settings are never changed.
5. **Painted-triangle remapping** (`--remap-paint` and `--remap-model`):
   port of the Go/Prusa/Bambu recursive triangle paint bitstream, supporting
   split triangles, extended filament IDs and both `paint_color` and
   `mmu_segmentation` on model triangle XML. Unpainted regions and non-paint
   metadata are left unchanged. Bounded XML/bitstream input, not full 3MF I/O.
6. **Object and plate model settings** (`--rewrite-model-settings`):
   port of Go's metadata-extruder ID remapping and virtual `filament_maps`
   rebuilding for 1..64 slots. XML entities, namespaces and non-target
   attributes are preserved; invalid XML and directives are refused.

Cache cleanup now supports Go's `stripSliceCache(source, destination, preserved)`
contract, including named process-setting overrides supplied through repeated
`--preserve` options. It preserves existing process keys, expands the settings
array to the number of filament slots plus two and keeps unrelated settings.
The new slot-planning and material-mapping entrypoints are **read-only
planning helpers**. They do not discover or authorize installed target
presets and do not convert a 3MF file. Full printer-specific 3MF conversion,
profile loading/verification and slicer compatibility remain in Go.

No physical printer is contacted and no print command is generated or sent.

## Build and test

```sh
cd rust/3mf-probe
cargo test --locked
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Example read-only inspection:

```sh
cargo run --locked -- ../../tests/fixtures/bambu-five-colors.3mf
```

Expected response contains:

```json
{"ok":true,"facts":{"vertices":4,"triangles":4,"slice_caches":3,"printer_model":"Bambu Lab A1"}}
```

Example cleanup into a *different*, not-yet-existing file:

```sh
cargo run --locked -- --strip-cache ../../tests/fixtures/bambu-five-colors.3mf /tmp/fixture-cleaned.3mf
```

To mark specific process settings as preserved during cleanup, supply one
`--preserve` argument for each setting, before the source and destination:

```sh
cargo run --locked -- --strip-cache --preserve filament_flow_ratio --preserve layer_height \
  ../../tests/fixtures/bambu-five-colors.3mf /tmp/fixture-preserved.3mf
```

A successful cleanup prints e.g. `{"ok":true,"removed":3}`. It does
not modify the source and cannot overwrite an existing destination. The
source settings remain unchanged; only the new output receives override tags.

To test the new *read-only* material operations, create a local JSON request:

```json
{"filaments":[{"id":1,"color":"#FFFFFF","type":"PLA"},{"id":2,"color":"#FFFFFF","type":"PVA"}]}
```

Then run `cargo run --locked -- --plan-slots path/to/request.json`.
This produces a distinct virtual slot for each ID and marks PVA as support.

For `--map-materials`, provide a JSON object containing both a `bundle`
(snapshot of the existing installed Go targetBundle, including `profile`,
`filament_profiles`, `project_settings` and `material_settings`) and a
`filaments` array. The response contains `project_settings` only. This
command **does not resolve installed presets, convert projects or write
any output file**. Both JSON commands limit input to 8 MB.

To test the independent XML conversion stages, provide one of the following
JSON requests in a local file and run the corresponding command:

```json
{"samples":["4","8","4882"],"mapping":{"1":2,"2":1}}
```

```sh
cargo run --locked -- --remap-paint path/to/request.json
```

For `--remap-model`, use `model_xml` (UTF-8 XML) instead of `samples`,
alongside the same mapping. The output contains rewritten `model_xml`.
For the separately bounded `--rewrite-model-settings` command:

```json
{"model_settings_xml":"<config><metadata key=\"extruder\" value=\"2\"/></config>","slots":4,"mapping":{"2":1}}
```

All three commands only return JSON; they do not write or replace source
3MF files. Raw model XML is bounded to 8 MB; model-settings XML is bounded to
8 MiB and disallows directives/DTD. The standalone paint codec rejects malformed
hex and excessive tree recursion. These transformations are **not wired into a
full Rust converter**; source 3MF validation, installed printer profile identity,
the other XML/ZIP transforms, and actual conversion publication remain in Go.

## Verify Rust output against the real Go implementation

Build the Rust CLI and opt into all Go differential tests:

```sh
cargo build --locked --manifest-path rust/3mf-probe/Cargo.toml
cd engine
SLICEBRIDGE_RUST_PROBE="$(pwd)/../rust/3mf-probe/target/debug/slicebridge-3mf-probe" \
  go test -count=1 ./bridge -run '^TestRust.*Parity$' -v
```

The existing Go code is the baseline. Three archive tests compare 3MF facts,
cache removal and preserved process settings. Two new mapping tests compare
Go sourceSlotPlan and bindNativeFilamentSettings directly against their Rust
implementations, including failure acceptance. Each of the five suites runs
on 11 checked-in sample 3MF fixtures; the mapping suites also cover 20
synthetic success/failure cases, including mixed materials and hardware-owned
settings. Unchanged archive payloads are compared byte-for-byte, XML
semantically and JSON structurally. Ordinary Go tests skip the differential
checks unless `SLICEBRIDGE_RUST_PROBE` is set.

**Verified:** on 13 November 2026, 50 Rust tests, Rust formatting/lint
checks, and the complete Go engine suite passed on MacMiniOffice.local.
The previously verified 83 differential cases now have **860 extra
comparison cases**: 855 real paint-hex corpus comparisons (285 encoded
triangle samples, three mappings), 1 XML triangle-paint comparison and
4 3MF model-settings tests against Go-converted packages. That is **943**
Mac Go/Rust comparison cases across the pilot so far. This evidence is
**not proof of full cross-slicer conversion, print quality, or release readiness**.

## Testing against Go-converted 3MF outputs and real installed material values

The opt-in `TestRustSanitizeConverted3MFParity` tests a separate, user-safe
path: the real Go upstream converter creates Bambu-to-U1 3MF output with
1, 4 and 6 virtual material slots, after which the experimental Rust tool
performs *only* cache sanitation and preserved process-setting updates.
Each resulting full ZIP is compared against the actual production Go output,
retained parts and metadata are checked, and all cached G-code must be absent.
All **3** cases passed. The production Go converter still owns every other
conversion stage; Rust is not an independent full converter yet.

`TestRustInstalledPresetSnapshotParity` accepts an optional directory of
JSON snapshots with `bundle`, `filaments`, `expect_accept` and
`provenance` fields. Supply `SLICEBRIDGE_NATIVE_MATERIAL_SNAPSHOT_DIR`
alongside `SLICEBRIDGE_RUST_PROBE` when running Go tests. On Chris's Windows
PC, selected parameters and SHA-256 hashes were read from **four real OrcaSlicer
Qidi X-Plus 5 filament preset files**; the user and installed files were not
modified. A scratch-only local snapshot was assembled on MacMiniOffice.local,
with a deliberately synthetic machine baseline. One scenario with complete
selected material fields passed Go/Rust mapping, and one scenario where raw
Generic PLA omitted a flow ratio was rejected by both. These **2** comparisons
passed. The raw vendor preset inheritance has *not* been flattened or fully
validated; this is real material-value evidence, **not** verified printer
compatibility. The scratch snapshots are **not checked into Git**.

A bounded **read-only independent Claude review** raised edge cases around
support-filament recognition, whitespace in material names and duplicate-ID
error ordering. These were checked against Go source: support classification
and material-name trimming already matched; added Go/Rust regression inputs
for both, and reordered the Rust duplicate-ID error to match Go. The Rust
material mapper retains a deliberate 64-slot input limit, unlike an isolated
Go binder called directly with an oversized, synthetic installed profile;
the real Go conversion plan separately enforces the same 64-slot bound.

## Independent parallel verification of model XML transforms

A separate scoped Claude Code worker implemented model-settings XML handling
and 16 tests in an isolated Git worktree. Its code was reviewed and copied
into the experimental branch only after passing the tests, strict lint and
semantic comparisons against **actual Go-converted 3MF model-settings XML**.
Three real Go-conversion scenarios (1, 4 and 6 virtual channels) and a fourth
non-identity two-filament slot swap matched Rust output. The Go converter
correctly rejects undeclared source extruders, so the integration fixtures
use valid declared slots; an excess-ID fallback is exercised separately by
Rust unit tests. Claude's code uses a restricted XML scanner that rejects
DTD/directives and unknown entity references, stricter than the Go parser.

The independent Rust paint codec was checked against every line in the Go
dependency's real-model paint corpus, with exact encoded result matches across
all 855 remaps. The rewritten triangle XML was separately parsed and compared
against the Go paint codec's output. The standalone XML commands **do not
rewrite the ZIP archive or create printable files**.

## Next steps

- Port remaining slice-info XML and project-settings composition with full
  differential validation before assembling a new Rust 3MF conversion pipeline.
- Test more real-world 3MF sources and adverse ZIP/XML edge cases.
- Validate complete, inheritance-flattened installed vendor machine/material
  bundles and user-permitted multi-material 3MF examples. Selected raw material
  values alone are not a verified installed-target profile.
- Add actual end-to-end 3MF conversion and printer-profile resolution in
  independently gated slices; refuse unmapped filament materials.
- Benchmark Rust versus Go only after file-level output parity is proven.
- Run the Windows binary on a Windows host and test generated 3MF files in
  real slicers before considering a user-visible integration.
- Consider Tauri 2 as a separate UI decision after compatibility testing.

Do not enable this crate in the current application, remove Go/Wails, or
publish release builds based solely on these tests. Dependency versions
are recorded in `Cargo.lock`; release licensing and platform verification
remain separate.
