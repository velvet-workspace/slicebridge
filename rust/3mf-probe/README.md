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
7. **Project-settings composition** (`--compose-project-settings`) exactly
   rebuilds the target material color/type/profile arrays, re-sizes existing
   filament and flush-volume arrays, and carries support overrides from an
   explicit Go-printer-profile snapshot. It refuses invalid slot counts.
8. **Slice/plate metadata** (`--rewrite-slice-info`): Claude Code separately
   authored an in-memory Go-equivalent XML rewrite that preserves independent
   plate material usage, aggregates mapped filament metres/grams, pads target
   slots and updates the target printer label. Integrated only after independent
   tests and comparison with actual converted Go 3MF outputs.
9. **Experimental complete ZIP assembly** (`--convert-archive`) accepts an
   explicitly supplied conversion plan (no discovery), converts model paint,
   object settings, slice/plate metadata and project settings, removes stale
   G-code and OPC references, and atomically publishes a brand-new 3MF file
   without overwriting anything. It validates source ZIP structure, rejects
   stale source-settings snapshots, undeclared paint/extruder source IDs,
   incomplete/extra/invalid filament mappings and mismatched printer slot
   capacities, then reinspects the staged converted ZIP before publication.

Cache cleanup now supports Go's `stripSliceCache(source, destination, preserved)`
contract, including named process-setting overrides supplied through repeated
`--preserve` options. It preserves existing process keys, expands the settings
array to the number of filament slots plus two and keeps unrelated settings.
The standalone slot-planning and material-mapping modes are read-only.
The **experimental ZIP assembler does produce complete 3MF files** from
an explicit, already vetted plan, but it **does not** verify or load the
real installed target printer, nozzle, material presets or hardware.
Only the production Go bridge is authorized to resolve those dependencies,
validate native profile compatibility and operate the installed application.

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

## Independent end-to-end Rust ZIP experiment (not production ready)

In addition to the standalone components, the opt-in
`--convert-archive plan.json input.3mf new-output.3mf` command accepts:

`profile` (Go printer.Profile snapshot, including exact machine settings and
filament slot capacity), `slots` (fully specified source material channels),
`slot_count` (equal to the profile capacity), `mapping` (one canonical source
filament ID per declared source filament, each destination limited to one
of the explicitly chosen slots), `source_settings` (exact source project
settings JSON), `supports` (boolean), and optional `preserved` (print
settings key list). The JSON plan is bounded to 8 MB. No target printer
profile is downloaded, discovered, installed or used to send printer commands.

On MacMiniOffice.local, the experimental independent Rust ZIP output matched
Go converted+sanitized output in **six separate conversion scenarios**:
1→4, 4→4 (with/without supports), 6→6 and 6→4 (with/without supports).
Checks cover every ZIP member, XML semantics, JSON settings, raw unchanged
payloads, the original source remaining identical and existing-output
non-overwrite. **Ten negative input cases** (source settings mismatch,
incomplete/extra/invalid source mappings, mapping into unallocated bays,
profile count mismatch, missing native material ID, empty material type,
undeclared default extruder, undeclared paint ID) are required to refuse
output. The experimental model is **not approved for print**.

Known fail-closed limitations: project and XML members over 8 MiB,
3MF projects without declared slice_info filament IDs, unsupported XML DTDs,
and source JSON number representation differences (e.g. 1 versus 1.0).
Vendor preset inheritance, target machine authenticity, slicing and real
filament/tool assignment are still owned by Go and require new end-to-end
validation. No installed printer, actual slicer import or physical print has
been tested with the Rust output.

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

## Native slicer import smoke check (PrusaSlicer, opt in)

The `TestRustFullArchiveExperimentalParity` differential test optionally
exports its six source/Go/Rust triplets (18 3MF files) to a **new, existing
scratch directory** when `SLICEBRIDGE_SLICER_OUTPUT_DIR` is set. It uses
exclusive creation and refuses to overwrite prior artifacts. By default
the files remain inside the disposable Go test directories.

On Windows Chris, **PrusaSlicer 2.9.6** was invoked with `--info --datadir`
against an isolated test-only data directory. It imported **all 18**
packages successfully. The six sets of input/Go/Rust models had identical
geometry statistics as reported by PrusaSlicer: dimensions, bounding box,
mesh facets, part count, manifold status and reported volume. All input
SHA-256 digests were unchanged. There was no G-code slicing, network
printer access, installed application change or user-preset modification.

The reproducible opt-in `TestRustNativeSlicerImportParity` runs the same
read-only import against saved scratch output pairs and automatically
compares full geometry stats. Set `SLICEBRIDGE_NATIVE_SLICER_EXE` to the
installed `prusa-slicer.exe` executable and point
`SLICEBRIDGE_SLICER_OUTPUT_DIR` to the 18 scratch files, then run the
Go bridge test. The test gives the native CLI a fresh isolated
`--datadir` directory.

### Pinned-source corpus and native-mesh outcome parity

A second opt-in suite, `TestRustPinnedBambuArchiveConversionParity`, converts
**all eleven** checked-in Bambu-style fixture 3MF archives using the new
standalone Rust ZIP pipeline and actual Go conversion, then checks every
archive member for semantic parity and source immutability. It optionally
exports an additional 33 files into the same *exclusive, scratch-only*
`SLICEBRIDGE_SLICER_OUTPUT_DIR` directory under `pinned-` names.

An isolated Windows PrusaSlicer CLI check opened **33 source/Go/Rust 3MF
files individually** and found consistent native outcomes for every trio:
**seven pinned sources imported successfully** with identical mesh metrics
after both conversions, while **four pinned sources** returned
"The 3MF file does not contain a valid mesh" *before conversion* and
both corresponding Go and Rust converted files returned the same error.
The reproducible `TestRustNativeSlicerPinnedCorpusParity` distinguishes
an expected source-invalid rejection from a new conversion regression:
a mismatch of accepted/rejected status fails the test, and successful
imports must match all reported geometry metrics. A bad original mesh is
never counted as a successful native import.

PrusaSlicer's separate `--export-stl` action exported the full-Go and
full-Rust output of each of the six synthetic conversion cases into
isolated scratch files. Every Go/Rust STL pair matched in SHA-256.
These six conversion cases share a tiny, nonmanifold synthetic mesh, so
the equal STL hashes cannot demonstrate varied real-model geometry QA.

**Important limitation:** these small synthetic QA meshes are not manifold
and have zero reported signed volume in the differential set. PrusaSlicer's
ability to read them does **not** prove printability or compatibility of
Bambu/Orca metadata, vendor-specific material profiles, multi-extruder
tool assignment, supports, tool changes or slicer preview. OrcaSlicer,
Bambu Studio, QIDI Studio and Snapmaker Orca were found installed but
have not yet passed an independent native import equivalence test.

## Live installed Snapmaker U1 / QIDI vendor profile milestone

The real `Snapmaker Orca 2.4.1` process preset
`0.20mm Standard @Snapmaker U1 (0.4 nozzle).json` and its
`fdm_process_common.json` parent have a *leading UTF-8 BOM* (EF BB BF).
The existing native/presets catalog reader refused these valid JSON
resources, preventing the target profile from resolving at all.
**The draft branch now removes only the exact leading three-byte BOM**
at all three vendor JSON entrypoints; malformed JSON, UTF-16, embedded
BOM and incomplete signatures remain rejected. Synthetic inheritance,
Orca-discovery and invalid-input regression tests passed on both
MacMiniOffice.local and Chris Windows PC. No installed OEM or user file
was changed.

After the patch, the **installed** `presets.LoadConfigured` resolver
successfully flattened Snapmaker U1 0.4 mm and QIDI X-Plus 5 0.4 mm
machine/process/material chains on Windows Chris, using the installed
authoritative code rather than hand-authored fallback profiles. Read-only
local snapshots were saved **only** in a scratch QA directory (not
tracked by Git). Snapmaker U1 used the canonical
`Snapmaker U1 (0.4 nozzle)` process/machine identity; the installed OEM
leaf contained a conflicting raw `MyToolChanger ... - Copy` setting
ID, correctly overridden in the resolved project identity.

The production **Go bridge** converted source files with these real
installed bundles (one and five source materials for each target:
**4/4 passes**, no printer operation). The converted 3MF project
settings preserved nine selected OEM-owned machine/script/identity
settings in **all four outputs**. This confirms that the earlier
observed differences from the *old built-in Go U1 template* do
not constitute script-loss in the installed-vendor conversion path.

A new opt-in integration test
`TestRustFullArchiveInstalledVendorProfileParity`, given
`SLICEBRIDGE_INSTALLED_BUNDLE_DIR` (explicit local resolved JSON
snapshots) and `SLICEBRIDGE_RUST_PROBE`, independently converted the
same two pinned source models to each of the two OEM targets using
the Go upstream converter and standalone Rust ZIP converter.
**All four full ZIP comparisons passed** on Windows, including
semantic XML/JSON and unchanged raw payloads; the test specifically
guards the real installed OEM start/end/tool-change scripts. This is a
lower-level equal-baseline conversion comparison and **does not
replicate native material binding or profile discovery in Rust**.

An explicit opt-in scratch-only exporter produced twelve 3MF files
(four source/Go/Rust triplets). PrusaSlicer 2.9.6 imported all
**12/12**; dimensions, facets, parts and volume matched across each
triplet, and input SHA-256 files remained unchanged. No target-slicer
UI preview, G-code generation or actual print was tested.

### Benchmark: integration-boundary cost, not a Rust speed claim

The committed `BenchmarkRustGo3MFArchive` compares *in-process*
Go conversion plus Go cache sanitization against a **new Rust CLI
process per source** including Rust sanitization. On tiny pinned 3MF
files, with Go benchmark `-benchtime=300ms`:

| Host | Go in-process | Rust subprocess | Model |
| --- | --- | --- | --- |
| Windows Chris / i9-14900K | 3.31–3.53 ms/op | 15.84–16.91 ms/op | one / five materials |
| MacMiniOffice.local / Apple M4 | 11.23–11.26 ms/op | 69.53–69.91 ms/op | one / five materials |

This does **not** show Rust is intrinsically slower: the Rust test
includes process startup, OS launch overhead and I/O; the Go test is
a direct library call. Different hosts and external/portable
toolchain configurations are not cross-host performance comparisons.
For a desktop migration, evaluate a persistent Rust process, native
library interface or another authorized integration after hardware
profile and slicer import gates close. Larger production-sized
3MF sources remain unbenchmarked.

### Follow-up: optimized Rust Release build, same subprocess boundary

Independently compiled the opt-in Rust executable with
`cargo build --release --locked` on both Windows and macOS, then
ran the native Rust test suite. `cargo clippy --release --locked
--all-targets -- -D warnings` passed on Mac. The **optimized**
Rust CLI still launches a process per conversion:

| Host | Go in-process | Optimized Rust CLI process | Input |
| --- | --- | --- | --- |
| Windows Chris / i9-14900K | 5.04–5.30 ms/op | 13.45–14.39 ms/op | one/five colors |
| MacMiniOffice / Apple M4 | 11.38–11.56 ms/op | 30.40–30.48 ms/op | one/five colors |

These are isolated, short-duration timings on tiny 3MF fixtures,
with run-to-run variability. Go and Rust use different invocation
boundaries, so the result **does not** measure intrinsic Rust vs Go
throughput; importantly, it **rules out** promising a speedup merely
by swapping the current Go library call for a per-conversion Rust
CLI. No installed application was changed or released.

### Stable source snapshot throughout conversion

The experimental Rust archive converter now captures a bounded
(250 MB maximum), immutable private source snapshot in the caller's
approved output directory, using exclusive file creation and cleanup.
It inspects the ZIP central directory, validates material references,
transforms model XML and settings, and compares output geometry
against **the same captured bytes**. The source path is not reopened
later in the pipeline. This prevents a source replacement during a
long conversion from causing different validation/conversion stages
to operate on different 3MF files. An independent Rust unit test
replaces the source path after snapshot creation and verifies snapshot
stability and automatic deletion; an empty source is rejected without
scratch leaks. On abrupt process termination, an orphaned partial
snapshot may need manual cleanup. Copying the snapshot adds I/O,
particularly on large projects; benchmark it separately before
adopting this pipeline.

### Streaming 3MF model XML beyond 8 MB

The full-archive converter now audits triangle paint references and
rewrites model XML using buffered `quick-xml` events, rather than
allocating an entire model XML byte array. Model entries are capped at
the existing inspector's **200 MB uncompressed model XML limit**;
the standalone `--remap-model` JSON API remains bounded to 8 MB.
The archive converter still requires a verified 250 MB maximum
compressed input and a complete explicit source filament mapping.

A Rust unit test streams an XML model containing more than 8 MiB of
many small vertex tokens and verifies precise paint transformation.
The opt-in `TestRustLargeModelArchiveParity` was strengthened from
its earlier **9 MiB repeated XML-comment fixture** to actual
**24,000 closed tetrahedra, 96,000 vertices and 96,000 triangles**,
with **8,778,159 bytes of model XML**. It uses two distinct
`slice_info.config` plate metadata sections and independently stale
cached artifacts for both plates. Real Go conversion and the
experimental Rust full-ZIP converter now produce semantically identical
3MF members; the source is unchanged, both metadata plates survive,
both slice caches are removed, no private partial files remain, and
final geometry counts are verified again by Go's inspector. This is
**synthetic mesh/multi-plate metadata stress**, not two separate real
slicer-layout plates or a print-ready manufacturing project.

Separately, Rust inspection now refuses an individual model member
larger than **200 MiB** from the ZIP central-directory size *before*
reading or allocating its XML event buffer. A passing Rust regression
creates a **201 MiB uncompressed** model member in fixed 1 MiB chunks
(with a small deflated scratch ZIP), then verifies both the library and
CLI reject it. This proves early **limit enforcement**, not successful
conversion of a real 200 MiB production geometry dataset.

### Post-snapshot and streaming Release benchmark (tiny QA models)

After adding source snapshotting and streaming XML, the optimized
`--release` CLI was rebuilt and measured again:

| Host | In-process Go + sanitize | Optimized Rust CLI + sanitize | Note |
| --- | --- | --- | --- |
| Windows Chris / i9-14900K | 4.96–5.60 ms/op | 14.69–16.73 ms/op | Two tiny pinned sources |
| MacMiniOffice / Apple M4 | 11.53–12.45 ms/op | 34.72–38.25 ms/op | Two tiny pinned sources |

These short tests include startup and file-system variance and
do **not** compare equivalent library boundaries. The numbers
are exploratory, not representative of large real-world projects;
the Go library remains the production performance baseline.

### Fail-closed number normalization and source rewrite verification

The full Rust converter now uses a recursive source-settings comparator
that accepts Go-equivalent JSON numeric representations (`1`, `1.0`,
`1e0`, `-0.0`), while requiring exact keys, types, array positions
and material mappings. Different values and numbers outside IEEE-754
safe-integer precision are still rejected if their representations
differ. The independent `TestRustNumericSourceSnapshotFullArchiveParity`
converts a synthetic 3MF ZIP containing mixed-format numbers and
matches actual Go ZIP output; separate negative tests reject stale
numeric values and ambiguous 2^53+1 integers.

The source snapshot now performs a second, byte-for-byte comparison
against the same originally opened source file descriptor after
copying (up to 250 MB). This rejects same-size in-place writes
that the previous file-length check missed. A deterministic Rust
test overwrites the same-length source after capture and checks
verification refuses it; more concurrent write fault injection
remains pending. It costs an extra read of source and snapshot.

### Native Snapmaker Orca geometry import — isolated Windows CLI

The installed `Snapmaker Orca` CLI supports `--info` with a
temporary `--datadir`. On Chris Windows, two Go-created U1
and two Rust-created U1 OEM 3MF packages returned the same mesh
dimensions, facet counts and volumes (2 pairs, 4 files).
An opt-in repeatable test
`TestRustSnapmakerOrcaInstalledOEMMeshImportParity` uses
`SLICEBRIDGE_NATIVE_SNAPMAKER_EXE` and an explicit
`SLICEBRIDGE_SLICER_OUTPUT_DIR` pointing at scratch OEM outputs;
it isolates APPDATA/LOCALAPPDATA and checks source checksums.

**Scope is geometry-only**: even these readable 3MF packages emitted
plate/XML warning logs, and the tiny fixture mesh is non-manifold.
This is not an interactive slicer project preview, material/tool-change
verification, G-code validation or a usable physical print. Source
Bambu and QIDI OEM files were not accepted by that *different vendor's*
Snapmaker CLI; QIDI exports specifically triggered a 1-vs-5
`filament_is_support` slot mismatch. This does **not** prove that
QIDIStudio would reject them. An isolated `qidi-studio.exe --info`
attempt returned no mesh metrics, so QIDIStudio's native import
remains **UNVERIFIED**.

### Installed OEM material-bound production-path parity (Windows scratch)

The opt-in `TestRustInstalledOEMProductionPlanParity` now verifies
the **real production Go `ConvertTarget` planning/publishing path**
against the standalone Rust 3MF ZIP converter, using copied locally
resolved 0.4 mm Snapmaker U1 and QIDI X-Plus 5 OEM bundles.
Unlike earlier machine-only OEM parity tests, this gate calls the
native Go material binder and preserves verified source process
settings before composing the Rust request. It compares every 3MF
member semantically and verifies material-profile names, color
identity, per-material temperature/flow settings where provided,
unchanged machine-owned firmware/nozzle IDs and source immutability.

The six isolated scratch scenarios are: one-color Bambu source,
five-color Bambu source, and a synthetic **mixed PLA/PETG five-slot
painted source**, for each installed U1 and QIDI bundle.

- **5 successful Go/Rust full-ZIP matches**: U1 one/five-color,
  QIDI one/five-color and QIDI mixed PLA/PETG.
- **1 required safety refusal**: the installed U1 PETG material
  preset supplies `nozzle_temperature_range_high=270` but omits
  the material's lower `nozzle_temperature_range_low`, whereas
  the PLA preset has its own lower bound. The Go production
  planner correctly rejects the mixed case without creating a 3MF.
  Tests intentionally require this rejection; the converter MUST
  NOT guess a PETG limit or inherit an unsafe PLA value.
- Another real installed U1 material issue was resolved in the
  draft branch: a material preset may omit `filament_settings_id`
  or hold an empty placeholder while the fully resolved installed
  `profile.filament_profiles` explicitly names the native PLA
  and PETG profiles. Both Go and Rust now bind those IDs from the
  verified map, without substituting a generic material; missing
  native material identity still fails closed. Synthetic Go/Rust
  positive/negative regression cases pass.

Ten opt-in material-bound output artifacts (five Go/Rust pairs)
remain in private Windows scratch, not installed slicer folders.
The separate
`TestRustSnapmakerOrcaMaterialBoundOEMMeshImportParity` passed:
the installed Snapmaker Orca `--info` accepted all **4 new
material-bound U1 Go/Rust files** with identical mesh metrics.
This adds actual native CLI geometry-read evidence, **not**
slicer GUI material/tool readiness, an approved mixed U1 PETG
profile, valid slicing or a print-ready project. The QIDIStudio
native GUI import/tool preview also remains UNVERIFIED.

### Verified installed Snapmaker U1 0.4mm PLA/PETG leaves — subsequent fix

The **five pass / one safe refusal** results above were the
*historical, incomplete OEM base-profile snapshot*. A later
read-only inspection of installed Snapmaker Orca manufacturer JSON
established that the real `Snapmaker U1 (0.4 nozzle)`-compatible
`Generic PLA High Speed.json` and `Generic PETG.json` are
specific manufacturer *leaf* profiles, inheriting from their
respective `@U1 base` names, which lack critical material values.
The 0.4mm PETG leaf explicitly declares an OEM
**230–270 °C** nozzle temperature range, `filament_flow_ratio=0.95`
and max volumetric speed `12`. The PLA High Speed leaf supplies
**190–240 °C**, ratio `0.98`, and max volumetric speed `18`.
These values were **read from actual installed OEM files** and
never invented, injected into user settings or sent to a printer.

The draft native resolver now chooses these exact OEM leaf names
only when installed file provenance, inherited base, explicit
machine compatibility, material type and each material's
temperature/flow/speed values independently pass validation.
Explicit user-selected material overrides are untouched.
Negative tests reject wrong nozzle, type, parent/filename,
incomplete bounds, missing flow or speed and invalid temperatures.
`TestInstalledU1PETG04LeafProfile` passed against the actual
Windows manufacturer installation in read-only mode.

The new scratch-only verified bundle
`slicer-qa/native-profiles-u1-pla-petg-leaf-20261114`
contains a fresh flattened U1 source and an identical copied,
previously verified QIDI OEM profile. With this bundle,
**all six** `TestRustInstalledOEMProductionPlanParity`
scenarios passed as **full actual Go-versus-Rust ZIP matches**,
including U1 five-slot mixed PLA/PETG, with no invalid
per-material settings or changes to the source. The older
incomplete U1 PETG bundle still correctly triggers the
required safe-refusal regression. No installed profiles changed.

The first synthetic mixed mesh was not accepted by native
Snapmaker Orca `--info` for *either* Go or Rust output, although
PrusaSlicer reported mesh statistics for both. To separate
geometry from native material parsing, the native-import test
now clones the separately proven native-readable pinned Bambu
five-color 3MF, changes only the fifth declared filament from
PLA to PETG, and preserves every mesh/paint token unchanged.
The actual Go and Rust full ZIP outputs still match. The installed
Snapmaker Orca CLI **successfully loaded both** mixed PLA/PETG
outputs and reported matching mesh metrics in
`TestRustSnapmakerOrcaVerifiedPETGMixedLeafMeshImportParity`.
It remains **geometry-only**, not a slicer GUI material/tool
assignment check or manufacturing approval.

### Native GUI readback: Snapmaker PASS; QIDIStudio material import NOT accepted (2026-11-14)

The owner manually opened both centered, material-bound Go and Rust test
archives (`material-bound-gui-centered-20261114`) in the installed
native applications and supplied screenshots:

- **Snapmaker Orca U1 0.4 mm**: Go and Rust projects both displayed the
  same **five material profiles and ordered colors**, including a
  fifth Generic PETG, and a 10 mm test object *inside* the bed after
  fixing the synthetic source's initial out-of-bounds placement.
  The UI still warned about PLA/PETG on the same plate. No slice,
  physical tool changes, G-code, or printing was validated.
- **QIDIStudio 02.07.02.73, X-Plus 5 0.4 mm**: Go and Rust displayed
  identical small 10×10×10 mm / four-triangle geometry on the bed,
  but the **GUI showed only one filament**: the user's local
  `Velvet - kexcelled Marble PLA` profile. Neither displayed the
  expected four QIDI OEM Generic PLA slots plus fifth Generic PETG.
  The projects' actual ZIP `project_settings.config` each still
  contains all five installed OEM filament preset names, types,
  colors, temperatures and flow values. Thus *archive parity passed*
  but *native QIDIStudio multi-material GUI acceptance did not*.
  Do **not** treat this as an approved QIDI five-material project.

QIDIStudio's upstream release notes explicitly state that importing
3MF files it does not recognize as native QIDIStudio projects may
automatically replace the printer and filament presets (see
https://github.com/QIDITECH/QIDIStudio/releases/tag/v2.07.02.01 and
https://github.com/QIDITECH/QIDIStudio/releases/tag/v2.06.00.51).
Issue https://github.com/QIDITECH/QIDIStudio/issues/213 also reports
that some project settings overrides may be ignored without their
`different_settings_to_system` entry. Both are **candidate causes**,
not proof of the exact QIDIStudio 02.07.02.73 decision path; actual
QIDI material-box configuration and native-project provenance were
not verified. No user profile or installed slicer files were modified.

The Go production conversion now emits a native QIDI importer warning
for **every** QIDI source, including a single-filament 3MF. A single
physical spool still requires confirming the active local material
preset, physical filament, nozzle, temperature and flow: QIDIStudio
may substitute a saved local preset even when the converted archive
contains exactly one correct OEM material entry. With multiple source
materials, the warning additionally explains that the archived slots
are virtual, the owner's printer lacks a QIDI BOX, and the native
GUI may show only one local filament. The returned
`native_slicer_material_review_required` field is true for QIDI
regardless of source count; it does not authorize printing. Regression
tests preserve the target's hardware settings and fail-closed behavior
without inventing a QIDIStudio provenance marker or asserting an
unverified native importer fix.

The owner subsequently confirmed the X-Plus 5 **has no QIDI BOX** and
can feed **only one physical filament at a time**. This materially
changes the GUI acceptance criterion: the single local filament UI
is consistent with the available feeder, not itself proof that the
bridge lost material metadata. Conversely, a valid five-slot archive
cannot establish a physically usable five-material print on this
machine. The target resolver had also *incorrectly hardcoded four
physical toolheads for QIDI*. That count is now **one print nozzle**
(the optional BOX is a material feeder, not four physical nozzles);
the installed vendor bundle's four initial **virtual palette slots**
remain unchanged on disk, as do its firmware/nozzle settings. In the
*converted QIDI 3MF only*, the material palette is now sized to the
number of **actual source filaments** instead of outputting the
manufacturer bundle's four default entries unconditionally: one source
PLA produces one output PLA slot (no three synthetic white fillers);
five source materials still produce five distinct virtual slots. This
small in-memory-only correction is verified against Go/Rust full ZIP
parity with the installed read-only OEM QIDI profile.

The isolated `TestQIDIOnePhysicalToolKeepsSourceFilamentsVirtual`
now covers **one through six** original source material slots: it
reports one physical print tool, emits exactly the original count
and order in the virtual archive palette (no default filler slots),
and requires manual source-to-physical mapping only for multi-slot
projects. Every QIDI case requires native material verification,
including the single-spool import. Neither the installed vendor
presets nor the owner's active slicer session are edited.
The opt-in installed-OEM Go/Rust production-plan parity gate now
asserts the QIDI native importer review flag and warning for **both
single- and multi-material** input. All eight captured OEM U1/QIDI
one/five/mixed scenarios passed on Windows Chris (2026-11-14), with
full ZIP parity and original source immutability; this is not a native
QIDIStudio single-spool GUI preview test.

The installed OEM Go/Rust archive parity gate additionally verifies
`physical_toolheads=1` for QIDI and includes a **centered
one-material** pinned 3MF test source, whose mesh is preserved but
whose build item is translated into the printable bed. The scratch
Go/Rust outputs were subsequently examined in the native QIDIStudio GUI.

### QIDIStudio single-filament GUI readback: geometry PASS, material FAIL (2026-11-14)

The owner supplied two native QIDIStudio screenshots, showing the
experimental **Rust** (`bound-rust-qidi-centered-one.3mf`) and actual
**Go** (`bound-go-qidi-centered-one.3mf`) converted projects.
Both native windows show **X-Plus 5, nozzle 0.4 mm**, one displayed
filament **Generic PETG**, and model info **10 × 10 × 10 mm,
166.667 mm³, four triangles**. The model is visibly located inside
the displayed build plate. These are native GUI project/geometry
smoke checks, not slicing or printable-placement certification.

Independent read-only ZIP inspection of the exact corresponding
scratch artifacts in
`slicer-qa/material-bound-qidi-single-exact-palette-20261114`
confirmed:
- The original source declares **PLA, red `#FF0000`**.
- **Both** Go and Rust converted 3MF archives declare **PLA,
  `#FF0000`**, manufacturer material ID
  `Generic PLA @Qidi X-Plus 5 0.4 nozzle`, printer identity
  `X-Plus 5 0.4 nozzle`, 0.4 mm nozzle, 220 °C nominal,
  190–240 °C allowed range, flow ratio 0.98, and
  maximum volumetric speed 14.
- The owner-visible native QIDIStudio UI instead displayed
  **Generic PETG** for *both* converted projects.
  Thus the observed native material/preset display **does not
  match the PLA declared in either output 3MF**.

Package SHA-256 receipts:
- Source `2DB3A69B04F5F317EB0BB55BCA951E0FECE3EF72698918EA603E121F707F9EEC`
- Go `83A71FF0C3946BA9F198BF0AA23171C4965536D2D72B1E17A8E8CE1C616056B9`
- Rust `DD351B279320425DA4D63B0142D41675947A10C9009F032104B22DE8A3583896`

Owner-supplied screenshot SHA-256 (screenshot images are **not** committed):
- Rust `image(20261114-071709).png`:
  `7D43691CE458CB199A772B8F9F1C5AD85EB902F23683AAC0446AAEBC5D3842AF`
- Go `image(20261114-071721).png`:
  `0979FDC03F214C66D63CF222DB515BA9290A6C5C6C8A417B259DDC1C6BC7EE38`

**Acceptance decision:** native project/geometry smoke **PASS** for
both engines; **native QIDI material preset parity FAIL** for both.
The screenshots alone do not establish whether QIDIStudio import rules,
a persisted user-local preset or another GUI state caused the PETG
selection. Do not silently override the source material, invent a
vendor-native marker, claim slicer compatibility, generate G-code,
slice, print or merge/deploy the Rust draft.

This evidence also exposed a native SliceBridge UI gap: the backend's
`native_slicer_material_review_required` flag was not part of the
result-card status. The experimental branch now displays a visible
bilingual QIDI material-review warning and uses **Review required**
rather than green success, even for a single filament. All **202**
native Node UI tests passed on Windows after using its verified
`C:\Python311\python.exe` for the pinned visual-diff helper.

### Repeat QA: centered 4×PLA + 1×PETG for QIDI and Flashforge (2026-11-14)

The owner clarified that **Generic PETG was the local QIDIStudio preset
in active use during a live PETG print** when the previous one-filament
GUI screenshots were taken. This is a plausible explanation for the
local PETG display overriding PLA from the imported 3MF; **no controlled
native GUI experiment has established that this always occurs**. No
slicer session or running printer was interrupted for this repeat QA.

An opt-in `TestQIDIFlashforgeMixedFiveCenteredRustParity` now verifies
two additional machine-specific, exclusive-create scratch source 3MFs
based on the previously pinned, native-readable small tetrahedron.
Both sources retain five distinct virtual filament slots: PLA red
`#FF0000`, green `#00FF00`, blue `#0000FF`, yellow `#FFFF00`,
and PETG white `#FFFFFF`. The original editable mesh remains a
**four-triangle tetrahedron**, so the five palette entries do **not**
prove five separately visible painted faces or a printable five-tool
object.

- **QIDI X-Plus 5 0.4 mm:** verified installed 320×320 mm bed;
  translated 10 mm model to X/Y `155..165`, centered at `160`.
  One physical tool and five **virtual** slots; mapping, native
  material import and local preset review remain mandatory.
- **Flashforge Creator 5 Pro 0.4 mm:** freshly resolved installed
  Flash Studio Desktop OEM machine/profile; 256×256 mm bed;
  model at X/Y `123..133`, centered at `128`. Four physical
  tools and five virtual source materials; a fifth material cannot
  be assumed to have automatic tool assignment. Manufacturer
  PLA/PETG presets were resolved independently, retaining separate
  temperature, flow and maximum volumetric speed settings.

For **both** targets, production Go conversion and experimental Rust
conversion completed, the full 3MF ZIP contents were compared
semantically (PASS), all five material types, colors and profile IDs
were checked independently, target-bed transform was preserved,
stale cached G-code was absent in converted outputs, and the two
source files remained byte-identical. No native Flash Studio GUI import,
new QIDIStudio GUI import, Slice, G-code generation or physical
printer operation was attempted.

The installed Creator 5 Pro **0.25 mm** PLA preset exposed a
manufacturer-profile conflict: nominal nozzle temperature `220 °C`
but `nozzle_temperature_range_high=210 °C`. A separate read-only
snapshot was recorded; **no 0.25 mm output was approved or included
in the deliverable** pending correction/independent validation.

Local machine-only scratch bundle (not committed, not a release):
`D:\\VelvetOps\\SliceBridge-RustPilot-20261113\\slicer-qa\\qidi-flashforge-mixed-five-20261114-run2\\SliceBridge-QIDI-Flashforge-Go-Rust-4PLA-1PETG-QA.zip`.
It contains six source/Go/Rust 3MF files, `READ-ME-FIRST.txt`
and `qa-manifest-20261114.json`, and was independently reopened and
checked for exactly eight entries. ZIP SHA-256:
`b369a7741e07d232f153ea839226606b4f4a7df4bf38ab7da29edfbe641e667e`.

### Owner GUI readback: Flashforge PASSES, QIDI foreign-project import (2026-11-14)

After opening the above generated 4×PLA + 1×PETG QA artifacts in the
actual target applications, the owner independently confirmed:
- **Flash Studio Desktop / Flashforge Creator 5 Pro:** Go and Rust both
  display all **five** source palette entries, matching each other.
- **QIDIStudio / X-Plus 5:** Go and Rust both display **only the active
  black PETG filament currently loaded for an ongoing print**, not the
  four PLA + one white PETG virtual source slots. Since the same
  user-visible mismatch occurs for both engines, it is **not
  demonstrated to be a Rust regression**. The on-disk 3MF material
  arrays remain exact, as shown by archive-readback tests.

**Vendor policy and importer source evidence:**
- Official QIDIStudio release notes for **v02.06.00.51 (2026-05-22)**
  and **v02.07.02.01 (2026-07-09)** explicitly document automatic
  replacement of the printer/materials when importing non-QIDIStudio
  3MF projects. https://github.com/QIDITECH/QIDIStudio/releases
- The vendor's public `src/libslic3r/Format/qds_3mf.cpp` importer
  (`_QDS_3MF_Importer::_handle_end_metadata`, lines 4071–4088)
  sets `m_is_qdt_3mf=true` only for a 3MF `Application` metadata
  value beginning with `QIDIStudio-`. Its `Model.cpp` loader
  (lines 427–459) distinguishes `From_QDS` from `From_Other`.
  Merely having the right `printer_model` and
  `filament_settings_id` JSON values is **not enough**.
- Read-only inspection of **an actual shipped QIDIStudio calibration
  project** at
  `C:\\Program Files\\QIDIStudio\\resources\\calib\\filament_flow\\flowrate-test-pass1.3mf`
  confirms the QIDIStudio `Application`, `QIDIStudio:3mfVersion`
  and QIDI namespace/relationship/header metadata. This is a
  genuine QIDIStudio *producer-format control only* (for X-Max 3),
  **not a valid X-Plus 5 machine template**.
- A new opt-in
  `TestQIDIExternalProjectOriginAndFiveFilamentPalette` independently
  decoded the 3MF model XML and all five material-type/color
  entries from the exact scratch Go and Rust QIDI artifacts.
  Both had `Application=""` and five correct source slots.
  Both are **external/foreign 3MF** under the documented vendor
  importer check (2/2 subtests PASS).

**Engineering implication:** the gap is the QIDI-native *project
creation/import contract*, not demonstrated loss of PLA/PETG slots in
the Go/Rust archive converter. Do **not** spoof a `QIDIStudio-`
creator marker on a file not actually authored by that application:
the producer's full project schema, material mapping and native GUI
reopen behavior remain unverified. A truthful native-project
serialization/round-trip integration and independent QIDI GUI
readback are separate development gates. The experimental software
continues to warn that foreign QIDI 3MFs may be displayed with active
local materials; no QIDI BOX or multi-material printing was assumed.

### Bounded local Rust JSONL worker — persistent-process experiment

The optional `--convert-archive-stdio` mode processes a **finite**
sequence of JSON lines on stdin, each with `plan_file`, `source`
and a new `destination`. It responds with one flushed JSON result
per request, accepts at most 4096 requests and at most 8 MB per
request line, does not listen on a network port, and exits on EOF.
Every job still runs the experimental fail-closed archive converter,
including the immutable source snapshot, source-mapping audits and
exclusive destination creation. This mode is **not wired into the
installed application**, daemonized or deployed to the user.

The Rust `tests/stdio.rs` checks that malformed/incomplete requests
produce independent failures and the worker exits when stdin closes.
The Go `TestRustStdioWorkerFullArchiveParity` passes two complete
3MF conversions through one Rust process and compares both resulting
ZIP packages to the actual Go converter. It confirms a bad request
does not kill the worker, a duplicate output path is refused without
changing the file, and the source remains unmodified. It additionally
kills one separate worker **after a completed conversion response**,
then starts a fresh worker that successfully converts another source-
identical archive with full Go ZIP parity. This tests restart between
jobs, **not** an in-flight process crash, source-write race, partial
output repair or unattended worker lifecycle.

On two **tiny synthetic** Bambu 3MF fixtures, end-to-end
`-benchtime=300–350ms` optimized Release benchmarks show:

| Host | Go library + sanitize | Rust new CLI process | Rust reused stdio worker |
| --- | --- | --- | --- |
| Windows Chris / i9-14900K | 3.55–4.16 ms | 12.28–12.37 ms | **7.07–7.22 ms** |
| MacMiniOffice / Apple M4 | 11.01–11.57 ms | 34.97–36.08 ms | **23.68–25.04 ms** |

The persistent worker avoids per-conversion process startup,
improving Rust's own CLI time by approximately 30–42% in this
microbenchmark, but **is still slower than the Go in-process
baseline**. This is a controlled, opt-in boundary prototype,
not a justification for production migration or a language
throughput ranking. Benchmark real multi-plate and large sources,
memory, worker restart/timeout/failure recovery, and in-process
Rust integration before choosing a runtime architecture.

## Next steps

- Expand the new full-Rust 3MF ZIP comparison against real, multi-material
  projects and full, inheritance-flattened vendor profiles (with authorization).
- Add independent verification of installed target machine/nozzle identity,
  printer safety policies and source settings snapshots before any application
  integration. Reject an unknown printer or filament rather than assuming
  hardware compatibility.
- Stress-test streaming model XML at large production sizes (including
  a bounded 200 MB model) and real multi-plate source projects. JSON numeric
  equivalence now has passing synthetic Go/Rust end-to-end parity; extend
  numeric tests for real vendor settings and concurrent in-place source edits.
- Validate complete, inheritance-flattened installed vendor machine/material
  bundles and user-permitted multi-material 3MF examples. Selected raw material
  values alone are not a verified installed-target profile.
- Integrate target profile and nozzle validation as independently gated
  slices; retain the established Go authority until Rust safety parity is
  demonstrated for real installed slicer environments.
- Benchmark realistic multi-plate production-sized sources, including
  source-snapshot I/O and both persistent-process and in-process Rust designs.
- Validate native target-slicer imports and tool/material preview for
  Snapmaker Orca, OrcaSlicer, Bambu Studio and QIDIStudio in isolated sessions.
  PrusaSlicer geometry-only import has passed but cannot validate tool changes.
- Consider Tauri 2 as a separate UI decision after compatibility testing.

Do not enable this crate in the current application, remove Go/Wails, or
publish release builds based solely on these tests. Dependency versions
are recorded in `Cargo.lock`; release licensing and platform verification
remain separate.
