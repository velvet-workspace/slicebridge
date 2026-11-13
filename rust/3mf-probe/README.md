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

This intentionally supports only Go's
`stripSliceCache(source, destination, nil)` contract. Preserved setting
overrides and printer-specific 3MF conversion are **not** implemented in Rust.

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

A successful cleanup prints e.g. `{"ok":true,"removed":3}`. It does
not modify the source and cannot overwrite an existing destination.

## Verify Rust output against the real Go implementation

Build the Rust CLI and opt into both Go differential tests:

```sh
cargo build --locked --manifest-path rust/3mf-probe/Cargo.toml
cd engine
SLICEBRIDGE_RUST_PROBE="$(pwd)/../rust/3mf-probe/target/debug/slicebridge-3mf-probe" \
  go test -count=1 ./bridge -run '^TestRustArchive(Probe|Strip)Parity$' -v
```

The existing Go code is the baseline. The first test compares the four
archive facts; the second compares the number of removed cache items and
the entire resulting ZIP structure, checking XML references semantically
and all other retained parts byte-for-byte. Both run on the 11 checked-in
3MF fixtures. Go tests skip these cross-language checks unless
`SLICEBRIDGE_RUST_PROBE` is explicitly supplied.

**Verified:** on 13 November 2026, the opt-in Go comparisons passed for
all 11 fixtures in both tests, on MacMiniOffice.local using a separate,
external-volume toolchain. These are test fixtures, **not proof of
cross-slicer conversion, print quality, or release readiness**.

## Next steps

- Test more real-world 3MF sources and adverse ZIP/XML edge cases.
- Add preservation of setting overrides, with differential output tests.
- Benchmark Rust versus Go only after behavior is demonstrably equivalent.
- Consider Tauri 2 as a separate UI decision after compatibility testing.

Do not enable this crate in the current application, remove Go/Wails, or
publish release builds based solely on these tests. Dependency versions
are recorded in `Cargo.lock`; release licensing and platform verification
remain separate.
