# SliceBridge Rust / GPUI — installed OEM integration spike

This is an experimental independent Rust-native GPUI + GPUI Kit desktop app.
It does not contain a Go/Wails runtime, replace the stable app, generate
G-code, initiate slicing, or contact a printer.

## Implemented

- GPUI GUI: select 3MF, inspect geometry, choose an installed OEM printer,
  cycle through installed PLA/PETG material presets, choose destination,
  convert, cancel, read status and open converted output in an OEM slicer.
- One Rust Registry drives the GUI, JSONL CLI, MCP stdio and local attach.
- Native OS drag-and-drop zone selects one 3MF project or stages 2-32 files
  for batch; the GPUI window also provides recent-project open/forget controls
  and a scrollable per-file batch table with phases, receipts and errors.
- GUI file/folder pickers and native drops invoke the same batch.stage and
  batch.directory commands as CLI, MCP and attach. batch.start accepts either
  explicit paths/output_dir or the staged Registry selection. Jobs remain
  sequential/cancellable with deterministic, non-overwriting destinations.
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

Recent project history is session-local and agent-visible: projects.list,
projects.forget {"path":"..."}, and projects.clear. source.select updates it only
after successful 3MF inspection; no project file is modified or deleted.

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

Batch using the same *selected installed OEM printer/nozzle* (preselect a
3MF source and use profiles.select first):

    {"command":"batch.start","args":{"paths":["C:\\scratch\\one.3mf","C:\\scratch\\two.3mf"],"output_dir":"C:\\scratch\\converted"}}
    {"command":"batch.status","args":{}}
    {"command":"batch.cancel","args":{}}

The GUI and agents can instead stage a batch with:

    {"command":"batch.stage","args":{"paths":["C:\\scratch\\one.3mf","C:\\scratch\\two.3mf"]}}
    {"command":"batch.directory","args":{"path":"C:\\scratch\\converted"}}
    {"command":"batch.selection","args":{}}
    {"command":"batch.start","args":{}}

batch.start returns immediately; batch.status reports phase, completed/total,
progress_percent (whole completed files, not per-mesh estimates), active index,
and each file's source/output/phase/receipt. Terminal failures/cancellations
mark remaining queued items skipped rather than implying they succeeded.
At most 32 files; source filenames must be distinct at the output filename
level, files must be nonempty valid 3MF archives, and the output directory
must already exist. It writes <stem>-slicebridge-<target>-<nozzle>.3mf;
the nozzle decimal is expressed as an underscore (0_4 for 0.4 mm).
If a file fails, the batch stops and previous successful outputs remain.
Cancel stops the active worker and skips future files. No output file is
overwritten, including on a restarted batch. Slicing and printing remain
out of scope.

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

Windows batch QA passed 2/2 installed U1 OEM files, 3/3 installed
QIDI OEM files (including 80K triangles), and a 4/4 batch with 240K
and 960K triangle synthetic stress inputs (all original SHA unchanged).
Separate streamed XML geometry verification compared source/output vertex
coordinates and triangle vertex indices exactly on both large fixtures.
The 960K-triangle batch stress case took roughly two minutes on the QA
workstation; timing is not a performance guarantee. Tested refusal to
overwrite an existing output, source selection while active and live
batch cancellation. The running native GPUI window showed an agent-triggered
completed 2/2 batch without manual UI clicks.

Still incomplete: broader vendor models and Orca forks, deep slicer UI
acceptance at 0.2/0.8 mm, advanced tables and docking, actual drag-and-drop,
precise per-mesh progress percentages, localization, Linux, and printed-part
qualification. macOS requires this branch's next independent build/GUI QA
after the latest batch work.

Do not merge main, replace stable installation, slice, generate G-code or
control printers without applicable user authorization.
