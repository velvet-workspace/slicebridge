## SliceBridge Native — experimental dev.10 (0.4.0-beta.4.dev.10)

The native desktop workflow now supports:
1. **Saved per-printer preferences**: actual physical target nozzle
   (Auto/0.2/0.4/0.6/0.8, or 0.25 on Creator 5); preferred
   locally-installed slicer (QIDIStudio or OrcaSlicer on applicable systems);
   optional material-specific native filament presets for PLA/PETG/ABS/ASA/TPU.
   Preferences are validated against the real installed vendor profile data,
   stored as an atomic local JSON file in the OS user config directory
   (Windows: `%AppData%/Velvet Factory/SliceBridge/settings.json`;
   macOS: `~/Library/Application Support/Velvet Factory/SliceBridge/settings.json`).
   No preferences or filenames are sent to any cloud service.
   **Physical nozzle** settings are operator-specified; the tool cannot
   probe your actual printer's hardware. Set them accurately before slicing.
2. **Detailed per-output comparison** of source/target nozzle,
   source vs converted 3MF object/instance/plate counts where available,
   filament slot count, materials and colors, validated triangles,
   stale slice caches removed and preserved print keys. A complete batch
   report can be exported to local JSON. Source 3MF files missing nozzle
   metadata are explicitly shown as assumed 0.4 mm.
3. **Open output in its own installed slicer** after conversion.
   Only successful outputs from the current app session may be opened,
   and executable paths come from hard-coded, locally detected applications,
   never from arbitrary UI-supplied paths.
4. **Multiple source projects / folders**: select several 3MF files or
   recursively add a folder via native dialogs or OS drag/drop. Deduplicate,
   skip recognizable previously generated outputs during folder traversal
   only if the original source file exists as a sibling, except the already
   reserved generic `PRUSA-` and historical `ORCA-` export prefixes (to avoid discarding
   real user projects named `H2D-benchy.3mf` or `U1-bracket.3mf`). Do not
   follow symlinks; refuse batches above 200 source projects. Explicitly
   selecting a converted 3MF is still allowed. Output names use the physical
   printer model first (`U1-`, `Creator5Pro-`, `Neptune4Pro-`), never `ORCA-`;
   installed targets append physical nozzle and stable target ID (e.g.
   `Creator5Pro-0.4-SB0123456789-project.3mf`) to distinguish variants.
   The generic unbound Prusa export keeps `PRUSA-`. For each project, conversions run independently;
   projects process one-by-one to bound memory and give incremental progress.

### Window behavior / desktop interaction

- OS-native title bar and resize corners. The app no longer automatically
  resizes or recenters itself on language switch, status changes or results.
- Window width/height persist between launches in the same user profile.
  The native app always opens with a bounded minimum size; displays adapt.
- Top toolbars and bottom action bar stay fixed; input queue, printer
  selections and result cards scroll *inside* the workspace. A narrow-window
  fallback scrolls the workspace instead of clipping controls.
- Keyboard: Ctrl/Cmd+O add files, Ctrl/Cmd+Shift+O add folder,
  Ctrl/Cmd+, printer settings.
- The Hebrew RTL / English LTR toggle remains available at any time.
- A closeable on-demand GitHub release update notice is retained.
- Native asset JavaScript is plain JS, no bundled Node, npm, Electron or
  Chromium runtime. The output remains a single Windows executable or
  compact signed-local macOS app bundle.

### Source-to-printers UX state contract (implementation / QA)

The native UI flow is owned by `frontend/dist/app.js` and
`frontend/dist/my-printers.js`. This is a trace of shipped interface
logic, not a second router, a claim of physical printing or an assumption that
every catalogued printer is convertible.

| Observable state / trigger | Primary visible state and allowed action | Failure or recovery |
| --- | --- | --- |
| No source projects | Source queue stays compact; Convert is disabled | Add a local 3MF or folder; more than 200 projects is rejected |
| No supported destination selected | Convert is disabled; the target panel explains how to open My Printers | Choose an available profile in My Printers; saved but unavailable/stale IDs cannot be converted |
| Projects and supported targets selected | Convert is enabled and shows `files × printers` | Clearing either side returns to the disabled state |
| Batch running | Progress is based on **completed source projects**; queue, destination and settings edits are disabled | The backend may report per-output errors; never imply a print was started |
| Batch preflight error (`response.error`) | **Conversion failed** shows the backend error | Fix source/target inputs or settings and retry; preflight refuses the batch before processing projects |
| Mixed per-file/per-target outcome (`successes > 0` and `failures > 0`) | **Conversion partially completed** with accurate counts and warning emphasis (Hebrew/English) | Keep succeeded outputs and failed entries distinct; review failures and retry only as appropriate in the slicer/workflow |
| Structural conversion succeeded but physical mapping or tower clearance remains unverified | Result shows **Review mapping** and relevant cautions outside collapsed details | Operator checks tool/material assignments, G-code/layer preview and tower clearance in the destination slicer before printing |

Source/target and error behavior: `tests/main-destinations-ui.test.js`,
`tests/source-queue-ui.test.js`, `tests/my-printers-ui.test.js`.
RTL/LTR and compact desktop layout: `tests/source-queue-layout-qa.cjs`.
Those Chromium fixtures verify reachability and document workspace scrolling;
they do **not** certify native Wails, a real file dialog or printer operation.

### More source colors than physical tools (development behavior)

When a Bambu project has five or more original color/material slots, keep
all source channels independent in the converted 3MF project, along with
their color names/types and painted/object filament assignments. The
physical print nozzle count varies by printer model and is
independent of the number of project material slots. SliceBridge never
merges or discards source channels based on physical nozzle count. A
successful conversion is shown as converted without a noisy material-to-
toolhead warning: the user decides material/feeder mapping in the
destination slicer. The app never dispatches a physical print.
Valid installed vendor presets are still required for each material; do
not silently substitute a wrong temperature/material profile.
A successful structural round-trip of virtual 3MF slots is not proof
every slicer UI will retain extra channels after importing it.

### QA

Windows:
```powershell
cd native
$env:BRIDGE_TEST_INSTALLED='1'
go test -count=1 -v ./...
wails build -s
# Optional CSS/JS desktop smoke (requires existing dev Electron):
../node_modules/electron/dist/electron.exe tests/layout-smoke.cjs
```

macOS:
```sh
cd native
BRIDGE_TEST_INSTALLED=1 go test -count=1 -v ./...
wails build -s
```

The Electron-powered **layout smoke is development-only**: it exercises
the actual native HTML/CSS/JS with a stub bridge in 1100x750 and 780x580
windows, verifies bilingual direction switching does not change OS window
size, saved-settings modal and dismissal, multi-file rendering, scrolling
of 32-file and 128-result queues, and persistent action footer.
Production bundles still use system WebView2/WKWebView.

Real-slicer opening from the new Native UI and physical prints require
independent operator QA. Beta 2 of the *old* Electron app remains untouched.
