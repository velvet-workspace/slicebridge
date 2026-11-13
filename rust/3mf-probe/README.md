# SliceBridge Rust archive-probe pilot

**Status: experimental, read-only, and not wired into the application.** This
crate ports the bounded `inspectPackage` behavior from
[`engine/bridge/archive.go`](../../engine/bridge/archive.go) as the first
independently verifiable Rust migration slice.

It reads one editable 3MF archive and emits JSON with its geometry element
counts, source printer-model label, and stale slice-cache entry count. On
invalid input it emits `{"ok":false,"error":"..."}` and exits nonzero.

It does **not** create 3MF outputs, change filament/material mappings, call
external slicers, update the native Wails app, or send prints. Go remains the
production engine. The Rust crate is **not** a replacement converter yet.

## Local commands

From `rust/3mf-probe/`:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- ../../tests/fixtures/bambu-five-colors.3mf
```

Both source and destination data remain untouched; the binary only reads its
argument. The current JSON contract is:

```json
{
  "ok": true,
  "facts": {
    "vertices": 4,
    "triangles": 4,
    "slice_caches": 3,
    "printer_model": "Bambu Lab A1"
  }
}
```

## Run differential tests against Go

On a host that has **Go and Rust** installed, from the repository root:

```sh
cargo build --locked --manifest-path rust/3mf-probe/Cargo.toml
cd engine
SLICEBRIDGE_RUST_PROBE="$(pwd)/../rust/3mf-probe/target/debug/slicebridge-3mf-probe" \
  go test -count=1 ./bridge -run '^TestRustArchiveProbeParity$' -v
```

This optional Go test invokes the actual existing Go `inspectPackage` and
the Rust executable on repository 3MF fixtures. It compares acceptance and
the four extracted fields; it is skipped in ordinary Go test runs without
`SLICEBRIDGE_RUST_PROBE`. Error-message wording is intentionally not compared.

## Scope and acceptance gates

- Valid ZIP and editable 3MF structural inspection, with Go-aligned size,
  entry-count and geometry limits.
- Explicit rejection of unsafe paths and duplicate ZIP names. This needs
  special handling because the Rust `zip` crate indexes entries by name and
  otherwise collapses duplicate records.
- Rust tests cover valid, invalid, corrupt and adversarial synthetic inputs.
- The 11 checked-in `tests/fixtures/*.3mf` files are checked-in sample fixtures
  for manual CLI inspection; passing this check is not conversion parity.
- Cross-language acceptance requires the optional Go differential suite to
  pass on the same source revision, then real-slicer import and print-safety QA
  for any subsequent conversion logic.

**Next proposed Rust slice:** put the archive cache sanitizer behind a new
opt-in JSON CLI contract, test its full output ZIP structure against Go,
and only then consider a separately integrated Rust core/Tauri 2 shell.
Never remove the existing Go/Wails path as part of this pilot.

Dependencies are pinned in `Cargo.lock`. This experiment does not authorize
shipping new Rust binaries; release licensing, artifact signatures and actual
printer/slicer QA are separate gates.
