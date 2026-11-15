# SliceBridge Rust / GPUI — installed OEM integration spike

This is an experimental independent Rust-native GPUI + GPUI Kit desktop app.
It does not contain a Go/Wails runtime, replace the stable app, generate
G-code, initiate slicing, or contact a printer.

## Implemented

- GPUI GUI: select 3MF, inspect geometry, choose an installed OEM printer,
  cycle through installed PLA/PETG material presets, choose destination,
  convert, cancel, read status and open converted output in an OEM slicer.
- One Rust Registry drives the GUI, JSONL CLI, MCP stdio and local attach.
- OEM discovery resolves installed vendor machine/process/material inheritance
  on disk, read-only, including leading UTF-8 BOM where present.
- Windows OEM scope: Snapmaker U1 (Snapmaker Orca) and QIDI X-Plus 5
  (QIDIStudio), each with independently resolved and verified 0.2, 0.4,
  0.6 and 0.8 mm installed OEM machine, process and material profiles.
  No cross-vendor assumption or generated profile leaf.
- profiles.select validates the installed OEM resolution against the selected
  3MF source through the existing Rust-native source-owned conversion planner.
- profiles.materials lists real matching presets. material.select re-resolves
  and validates the conversion bundle; the selection changes output metadata.
- Existing slicebridge-3mf-probe Rust binary performs conversions using a
  transient verified OEM snapshot; original 3MF files are never overwritten.
- slicer.open_selected resolves the installed OEM application to open an
  already converted file; no slicing or printing command is issued.

## Build

    cargo test --manifest-path rust/slicebridge-alpha/Cargo.toml
    cargo clippy --manifest-path rust/slicebridge-alpha/Cargo.toml --all-targets --features desktop -- -D warnings
    cargo build --manifest-path rust/3mf-probe/Cargo.toml --bin slicebridge-3mf-probe
    cargo build --manifest-path rust/slicebridge-alpha/Cargo.toml --features desktop --bins

Keep slicebridge-desktop, slicebridge-alpha and slicebridge-3mf-probe together.
For a Windows executable add .exe to each name.

## Agent CLI and MCP

    slicebridge-alpha commands.list
    slicebridge-alpha profiles.discover
    slicebridge-alpha --stdio
    slicebridge-alpha --mcp
    slicebridge-alpha --serve 127.0.0.1:48638
    slicebridge-alpha --attach 127.0.0.1:48638 app.status

The running GUI listens at 127.0.0.1:48637, with the same Registry instance.
Keep a persistent CLI/MCP/attach connection for an end-to-end conversion.

Example JSONL command sequence:

    {"command":"source.select","args":{"path":"C:\\scratch\\project.3mf"}}
    {"command":"profiles.discover","args":{}}
    {"command":"profiles.select","args":{"target":"qidi","nozzle":"0.4"}}
    {"command":"profiles.materials","args":{"material":"PETG"}}
    {"command":"material.select","args":{"material":"PETG","preset":"Bambu PETG @Qidi X-Plus 5 0.4 nozzle"}}
    {"command":"conversion.start","args":{"output":"C:\\scratch\\converted.3mf"}}
    {"command":"conversion.status","args":{}}
    {"command":"slicer.open_selected","args":{}}

conversion.start is asynchronous. Wait for status completed before opening.
conversion.cancel can cancel a running Rust worker. bundle.load remains for
QA of a manually prepared and verified OEM snapshot.

## Verification and gaps

Windows QA on installed OEM resources and permitted 3MF fixtures passed
16/16 Rust conversions: U1 + QIDI, four physical nozzle variants each,
single-color and mixed 4xPLA+1xPETG cases. Every case checked profile
nozzle and printer identity, fifth PETG binding, geometry, removed slice
cache and unchanged source SHA.

In addition to archive inspection, verified native QIDIStudio and Snapmaker
Orca GUI import on installed applications for the 0.4 mm results:
QIDIStudio displayed X-Plus 5 with four Generic PLA slots and fifth
Bambu PETG; Snapmaker Orca displayed U1 with four Generic PLA High Speed
slots and fifth Generic PETG after "Open as project". Source SHA remained
unchanged. Native import did not create G-code or issue slice/print commands.
Real UI warnings still apply: QIDI warned about mixing PLA with PETG;
one tiny synthetic U1 mesh lay beyond the printable bed. These are
import-only receipts, NEVER physical print-readiness approval.

Still incomplete: broader vendor models and Orca forks, deep slicer UI
acceptance at the newly added 0.2/0.6/0.8 mm nozzle variants, advanced
tables and docking, batch/drag-drop, progress percentages, localization
and Linux platform testing. A previously verified 0.4mm build ran on
macOS; this new 8-nozzle commit needs macOS check and smoke.

Do not merge main, replace stable installation, slice, generate G-code or
control printers without applicable user authorization.
