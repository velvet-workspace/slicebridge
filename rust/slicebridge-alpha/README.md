# SliceBridge Rust/GPUI agent-first pilot

This is an **experimental, independent Rust desktop process** in a separate feature branch. It does **not** replace the stable Go/Wails application and does not slice, generate G-code, or contact printers.

## What works

- A native Windows GPUI + GPUI Kit window with file pickers, a project/printer/material summary, conversion start, cancellation, status and a user-selected target slicer launcher. This is a small functional frame, not the final rich editor/table/docking design.
- One Rust `Registry` in `src/lib.rs` powers all commands. The GPUI window, standalone CLI/JSON-lines, MCP stdio tools and the local loopback attach server reuse this logic.
- `source.select` and `source.inspect` reuse `slicebridge-3mf-probe::inspect_package`; `bundle.load` reuses `validate_native_bundle`.
- `conversion.start` launches the existing Rust-native `slicebridge-3mf-probe --convert-native-autonomous` executable as a sibling process. It refuses overwrite and supports `conversion.status` and `conversion.cancel`. This uses the proven conversion engine without copying it into UI code.
- `slicer.open` explicitly opens a previously converted file in a user-selected slicer executable. It never asks the slicer to slice.

## Build on Windows

Use the repository's available Rust toolchain:

```powershell
cargo test --manifest-path rust/slicebridge-alpha/Cargo.toml
cargo clippy --manifest-path rust/slicebridge-alpha/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path rust/3mf-probe/Cargo.toml --bin slicebridge-3mf-probe
cargo build --manifest-path rust/slicebridge-alpha/Cargo.toml --features desktop --bins
```

Place the three Rust binaries `slicebridge-desktop.exe`, `slicebridge-alpha.exe` and `slicebridge-3mf-probe.exe` together in a writable scratch directory when packaging. Avoid modifying installed production files.

## Commands and agents

```text
slicebridge-alpha.exe commands.list
slicebridge-alpha.exe --stdio
slicebridge-alpha.exe --mcp
slicebridge-alpha.exe --serve 127.0.0.1:48638
slicebridge-alpha.exe --attach 127.0.0.1:48638 app.status
```

The desktop process serves the same commands at `127.0.0.1:48637`. In persistent `--stdio` and MCP sessions, issue `source.select`, `bundle.load`, `conversion.start`, then poll `conversion.status`, and optionally call `slicer.open`. Example JSON-lines command:

```json
{"command":"source.inspect","args":{"path":"C:\\scratch\\source.3mf"}}
```

MCP methods are `initialize`, `ping`, `tools/list` and `tools/call`. MCP exposes the same named Rust commands and structured results/errors.

## Important acceptance gaps

1. The OEM `bundle.load` input must already be an *authoritatively resolved, complete* profile snapshot; Rust-native installed slicer discovery, inheritance and material/printer catalog selection are NOT yet implemented.
2. PLA/PETG buttons currently record **preview selection only**. They do not change the validated bundle or conversion parameters. Never substitute a guessed or incomplete OEM leaf.
3. The desktop window is still a pilot: no true large data table, docking framework, batch processing or progress percentage. The status reports job phases, cancellation and errors.
4. The UI is English-first with a clear localization slot; full string extraction, translated catalogs and RTL are later work.
5. Windows build/run has to be measured on the actual desktop; macOS and Linux need independent build/run smoke checks. No untested platform is claimed supported.
6. Integration parity with actual installed QIDIStudio/Snapmaker Orca and permissioned real 3MF sources is an acceptance gate beyond isolated synthetic fixture tests.

Never merge this spike into `main`, replace a stable installation, or slice, generate G-code, or print without separate owner authorization.
