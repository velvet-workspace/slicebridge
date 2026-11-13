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

## Verify Rust output against the real Go implementation

Build the Rust CLI and opt into all five Go differential tests:

```sh
cargo build --locked --manifest-path rust/3mf-probe/Cargo.toml
cd engine
SLICEBRIDGE_RUST_PROBE="$(pwd)/../rust/3mf-probe/target/debug/slicebridge-3mf-probe" \
  go test -count=1 ./bridge -run '^TestRust(Archive(Probe|Strip|Preserve)|SourceSlotPlan|NativeMaterialMapping)Parity$' -v
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

**Verified:** on 13 November 2026, 29 Rust tests, Rust formatting/lint
checks, and the complete Go engine suite passed. Go/Rust tests covered 55
archive/filament fixture comparisons (11 inputs across five suites),
23 synthetic slot/material mapping cases and five additional integration
checks below: 83 differential cases in total, on MacMiniOffice.local with
isolated toolchains on an external volume. These are **not proof of full
cross-slicer conversion, print quality, or release readiness**.

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

## Next steps

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
