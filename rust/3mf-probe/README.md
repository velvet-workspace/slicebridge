# SliceBridge: experimental Rust 3MF components

**Development only. Not integrated with the installed SliceBridge application.**

This small Rust crate incrementally ports two isolated operations from the
existing Go `engine/bridge/archive.go` implementation. The production
conversion engine and the Go/Wails desktop app remain unchanged.

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

Cache cleanup now supports Go's `stripSliceCache(source, destination, preserved)`
contract, including named process-setting overrides supplied through repeated
`--preserve` options. It preserves existing process keys, expands the settings
array to the number of filament slots plus two and keeps unrelated settings.
Printer-specific 3MF conversion and slicer profile translation are **not**
implemented in Rust.

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

## Verify Rust output against the real Go implementation

Build the Rust CLI and opt into all three Go differential tests:

```sh
cargo build --locked --manifest-path rust/3mf-probe/Cargo.toml
cd engine
SLICEBRIDGE_RUST_PROBE="$(pwd)/../rust/3mf-probe/target/debug/slicebridge-3mf-probe" \
  go test -count=1 ./bridge -run '^TestRustArchive(Probe|Strip|Preserve)Parity$' -v
```

The existing Go code is the baseline. The first test compares archive facts;
the second compares cleanup output; and the third repeats the cleanup with
preserved process settings and compares resulting settings as parsed JSON.
All three compare the number of removed cache entries and/or the resulting
ZIP structure, checking XML references semantically and unchanged payloads
byte-for-byte. They run on the 11 checked-in 3MF fixtures. Ordinary Go tests
skip these cross-language checks unless `SLICEBRIDGE_RUST_PROBE` is set.

**Verified:** on 13 November 2026, 21 Rust tests and all three opt-in Go
comparisons passed across all 11 fixtures (33 cross-language fixture checks),
on MacMiniOffice.local with Go/Rust toolchains isolated on an external volume.
The complete Go engine test suite passed. These fixtures are **not proof of
cross-slicer conversion, print quality, or release readiness**.

## Next steps

- Test more real-world 3MF sources and adverse ZIP/XML edge cases.
- Expand differential tests with real-world multi-material project settings and
  adverse input cases, beyond the checked-in sample fixtures.
- Benchmark Rust versus Go only after behavior is demonstrably equivalent.
- Consider Tauri 2 as a separate UI decision after compatibility testing.

Do not enable this crate in the current application, remove Go/Wails, or
publish release builds based solely on these tests. Dependency versions
are recorded in `Cargo.lock`; release licensing and platform verification
remain separate.
