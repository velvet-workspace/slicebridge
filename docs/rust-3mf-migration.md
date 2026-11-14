# SliceBridge — Rust 3MF converter migration gate (experimental)

Status: **opt-in engineering pilot, NOT a production migration**. Source branch:
feat/rust-3mf-main-sync-20261114, based on owner-accepted main
c46a6f1f3e30a2342e69da877dd3ef3f2a368b8f (0.4.0-beta.6.dev.3).

## What has actually moved

The Rust slicebridge-3mf-probe executable creates and sanitizes complete 3MF ZIP
packages: triangle paint, object/plate metadata, slice-info, project settings,
QIDI importer compatibility metadata, stale G-code/cache and OPC references.
It preserves unchanged payloads, atomically creates new output and refuses to
overwrite existing output.

The real Go ConvertTarget / Wails App.Convert call chain now has an opt-in Rust
full-archive writer in engine/bridge/rust_engine.go and engine/bridge/convert.go.
It executes Rust through an explicitly supplied absolute binary path in a
bounded 120-second child process with a machine-readable success receipt.
The output undergoes the production Go inspections (geometry, target identity,
stale G-code and atomic exclusive publication). Rust failures fail closed;
there is no silent Go fallback.

**Go policy has NOT moved:** Go still resolves installed OEM machine/nozzle and
material inheritance, validates the complete native target and safety-critical
temperature/flow settings, plans source slots and approved process overrides,
and publishes/reports the output. Rust consumes that validated snapshot; it
does not independently resolve manufacturer profiles. The Wails UI has no
Rust toggle and the installed release does not bundle this binary.

## Developer-only opt-in, scratch files only

Set two environment variables only in a disposable engineering test process:

- SLICEBRIDGE_CONVERSION_ENGINE=rust
- SLICEBRIDGE_RUST_ENGINE_BIN=(absolute path to a verified Rust release executable)

No engine selector or an explicit value of go keeps the installed Go default.
Any unsupported selector, missing Rust binary, invalid plan, failed conversion,
timeout or existing output causes explicit refusal. Never change system-wide
environment, installed desktop binaries or active print jobs for this pilot.

## Evidence and current limits

- Experimental PR #117 merged with verified current main into a scratch branch
  without conflict. Retired duplicate/obsolete tests; preserved quiet UI.
- Rust and Go native material-ID behavior agree with QIDI X-Plus 5's special
  installed profile map; non-QIDI targets use their real preset profile IDs.
- Five direct Go ConvertTarget vs Rust selectable ConvertTarget conversions
  passed full 3MF member parity, report identity, no-overwrite and source
  immutability checks.
- Native App.Convert created a real centered 4x PLA + 1x PETG source conversion
  for Snapmaker U1, QIDI X-Plus 5, FlashForge Creator 5 and ELEGOO CC2 using
  each engine; all four pairs passed full ZIP-member parity (semantic JSON,
  semantic XML/OPC, byte-exact untouched parts), source palette, plate/geometry
  counts, material IDs, QIDI importer metadata and no stale G-code.
- Eight production OEM U1/QIDI one/five/mixed scenarios passed independent
  Rust vs Go complete ZIP comparison using read-only installed profile bundles.
- Rust unit/release tests, full Go engine/native suites, Go vet, 198 native UI
  tests and version-contract checks are current QA gates. Mac QA and all CI
  must be read back independently.
- Windows i9-14900K tiny-fixture benchmark: in-process Go 3.48–3.91 ms/op,
  one-shot Rust CLI 12.27–12.38 ms/op, reused Rust stdio worker
  6.63–7.18 ms/op. Different integration boundaries and non-representative
  fixtures: do not claim Rust is faster on practical projects.

## Remaining gates — NOT ready to switch production default

1. Port or formally retain and document Go's production OEM profile resolver,
   nozzle/material safety validation, source planning and final publication.
   A fully standalone Rust core cannot currently run without a prevalidated
   Go plan.
2. Test real large multi-plate 3MF projects, bounded near-200 MiB model input,
   native target-slicer GUI imports with materials and placements, and complex
   failure/unsupported configurations. No slicing/printing.
3. Benchmark realistic projects, memory and total end-to-end latency; evaluate
   an in-process Rust interface vs persistent worker, in-flight worker crash,
   cancellation and restart, and no-duplicate exclusive publication.
4. Verify macOS ARM and Windows release builds with packaged Rust binary,
   license and security review, app rollout/rollback, exact-commit binaries
   and native Wails UI acceptance.
5. Expand adversarial ZIP/XML/JSON, symlink, race and source mutation tests;
   ensure original files, installed presets and existing outputs remain safe.

**Decision:** leave the owner-accepted Go/Wails app untouched. The experimental
Rust full-archive converter is connected and independently testable but is
not installed, not faster on current tiny fixtures, and not yet a complete
production-language replacement. Do not call the migration complete.
