//! One Rust command surface for the desktop and agent transports.
//! The proven 3MF implementation remains in slicebridge-3mf-probe.
pub mod batch;
pub mod browser_setup;
pub mod makerworld;
pub mod my_printers;
pub mod oem;
mod orca_profiles;
pub mod printer_settings;
pub mod protocol;
mod prusa;
mod prusa_condition;
mod prusa_ini;
pub mod updates;
use serde_json::{json, Value};
use slicebridge_3mf_probe::{
    inspect_package, inspect_source_nozzle, validate_native_autonomous_input,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
const MAX_RECENT_PROJECTS: usize = 16;
const ORCA_EXPORT_NOT_VERIFIED: &str = "dynamic Orca conversion is not verified in Rust";

// Go's installed Orca target list, exposed read-only without private profile
// paths or a conversion-ready claim. The full machine/process/material chain
// is already verified by orca_profiles::discover before a target is returned.
fn orca_discovery_reply(discovery: orca_profiles::OrcaDiscovery) -> Value {
    json!({
        "targets": discovery.targets.into_iter().map(|target| json!({
            "id": target.id,
            "title": target.title,
            "slicer": target.slicer,
            "vendor": target.vendor,
            "machine": target.machine,
            "model": target.model,
            "nozzle": target.nozzle,
            "physicalTools": target.physical_tools,
            "materials": target.materials,
            "ready": false,
            "reason": ORCA_EXPORT_NOT_VERIFIED,
        })).collect::<Vec<_>>(),
        "rejected": discovery.rejected.into_iter().map(|item| json!({
            "vendor": item.vendor,
            "machine": item.machine,
            "slicer": item.slicer,
            "reason": item.reason,
        })).collect::<Vec<_>>(),
    })
}

#[derive(Default)]
struct Session {
    source: Option<PathBuf>,
    // Changes even when an external CLI/MCP reselects the same path.
    source_revision: u64,
    facts: Option<Value>,
    recent_projects: Vec<PathBuf>,
    bundle_path: Option<PathBuf>,
    bundle_label: Option<String>,
    bundle: Option<Value>,
    installed_oem: bool,
    target: Option<String>,
    nozzle: Option<String>,
    selected_materials: BTreeMap<String, String>,
    output: Option<PathBuf>,
    phase: String,
    last_result: Option<Value>,
    cancel: Option<Arc<AtomicBool>>,
    batch: Option<Value>,
    batch_cancel: Option<Arc<AtomicBool>>,
    batch_staged: Vec<PathBuf>,
    batch_output_dir: Option<PathBuf>,
}

fn busy(state: &Session) -> bool {
    state.phase == "running"
        || state
            .batch
            .as_ref()
            .and_then(|batch| batch["phase"].as_str())
            == Some("running")
}

// Queue additions and explicit Clear results share the same session reset.
fn clear_conversion_results(state: &mut Session) {
    state.last_result = None;
    state.output = None;
    state.batch = None;
    if state.phase != "idle" {
        state.phase = "idle".into();
    }
}

// Saving preferences for the active target invalidates the old resolved bundle.
// Conversion cannot proceed until profile selection validates the new preferences.
fn invalidate_active_settings_target(state: &mut Session, target: &str) {
    if state.target.as_deref() != Some(target) {
        return;
    }
    state.bundle_path = None;
    state.bundle_label = None;
    state.bundle = None;
    state.installed_oem = false;
    state.target = None;
    state.nozzle = None;
    state.selected_materials.clear();
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedBatchOutput {
    path: PathBuf,
    target_nozzle: String,
    slicer_id: String,
}

// Match Go's generated-output ownership check: the requested source and target
// must resolve to a completed receipt in THIS registry session. Never trust an
// arbitrary output path supplied by the frontend or an MCP caller.
fn recorded_batch_output(
    batch: &Value,
    source: &str,
    target: &str,
) -> Result<RecordedBatchOutput, String> {
    let items = batch["items"]
        .as_array()
        .ok_or("no batch output is available in this session")?;
    let item = items
        .iter()
        .find(|item| item["source"].as_str() == Some(source))
        .ok_or("source was not generated in the current batch session")?;

    let (phase, receipt) = if let Some(targets) = item["targets"].as_array() {
        let selected = targets
            .iter()
            .find(|entry| entry["target"].as_str() == Some(target))
            .ok_or("file was not generated for this printer in the current session")?;
        (selected["phase"].as_str(), &selected["receipt"])
    } else {
        if batch["target"].as_str() != Some(target) {
            return Err("file was not generated for this printer in the current session".into());
        }
        (item["phase"].as_str(), &item["receipt"])
    };
    if phase != Some("completed") {
        return Err("batch output is not completed for this printer".into());
    }
    let output = receipt["output"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .ok_or("completed batch receipt has no generated output path")?;
    let target_nozzle = receipt["target_nozzle"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("completed batch receipt has no verified target nozzle")?;
    let slicer_id = receipt["slicer_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("completed batch receipt has no verified slicer identity")?;
    Ok(RecordedBatchOutput {
        path: PathBuf::from(output),
        target_nozzle: target_nozzle.to_owned(),
        slicer_id: slicer_id.to_owned(),
    })
}

fn validate_recorded_3mf(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("invalid output path".into());
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
    {
        return Err("choose a generated .3mf file".into());
    }
    let metadata = fs::metadata(path).map_err(|error| format!("read generated output: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 250 * 1024 * 1024 {
        return Err("generated output must be a regular .3mf file up to 250 MB".into());
    }
    Ok(())
}

#[derive(Clone, Default)]
pub struct Registry {
    state: Arc<Mutex<Session>>,
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("commands.list", "Discover typed Rust application commands"),
    (
        "printers.catalog",
        "Read exact 603-model offline stable My Printers catalogue",
    ),
    (
        "printers.installed_prusa",
        "Read installed PrusaSlicer user presets, all conversion-unverified",
    ),
    (
        "printers.installed_orca",
        "Read fully resolved Orca-family profiles, all conversion-unverified",
    ),
    (
        "settings.get",
        "Read validated stable printer settings or defaults without modifying installed settings",
    ),
    (
        "settings.recover",
        "Explicitly confirm backing up malformed preferences and restore alpha-only defaults",
    ),
    (
        "settings.window.remember",
        "Remember valid native window size only in isolated GPUI alpha settings",
    ),
    (
        "settings.developer.set",
        "Persist verified Developer Mode flag only in isolated GPUI alpha settings",
    ),
    (
        "settings.options",
        "List source-locked nozzle, installed slicer and verified material choices",
    ),
    (
        "settings.save",
        "Save one validated printer's preferences only to isolated GPUI alpha settings",
    ),
    (
        "printers.get",
        "Read saved My Printers inventory without modifying stable Wails settings",
    ),
    (
        "printers.save",
        "Validate and save up to 32 printer memberships in isolated GPUI alpha settings",
    ),
    (
        "updates.version",
        "Read installed SliceBridge version without network or mutation",
    ),
    (
        "updates.check",
        "Check only official GitHub Releases on demand, without downloading or installing updates",
    ),
    (
        "updates.page",
        "Construct allowlisted GitHub release/build page link without opening it",
    ),
    (
        "browser.setup.status",
        "Read-only inspection of the alpha app-link association; never register a protocol",
    ),
    (
        "browser.setup.store",
        "Read the source-locked approved browser store listing, empty until published",
    ),
    (
        "browser.setup.page",
        "Return only the selected browser extension manager page; never open it",
    ),
    (
        "browser.setup.prepare",
        "On a Developer Mode user gesture, prepare official bundled extension files only in GPUI Alpha config",
    ),
    (
        "browser.setup.open",
        "On explicit Developer Mode user click, open allowlisted manager or MakerWorld mode URL in selected browser",
    ),
    (
        "makerworld.origins",
        "Read bounded existing OS provenance for up to 200 local 3MF files",
    ),
    (
        "makerworld.link.preview",
        "Validate a trusted Bambu/MakerWorld deep link without downloading or opening it",
    ),
    (
        "projects.list",
        "List recently opened 3MF projects in this session",
    ),
    (
        "projects.forget",
        "Forget a recent project without changing its file",
    ),
    (
        "projects.clear",
        "Clear recent projects without changing files",
    ),
    (
        "batch.stage",
        "Stage up to 200 source 3MF files, matching the stable queue",
    ),
    ("batch.directory", "Select existing batch output folder"),
    (
        "batch.clear",
        "Clear staged source files without changing source files",
    ),
    (
        "batch.remove",
        "Remove one source from the staged queue without deleting the original file",
    ),
    (
        "results.clear",
        "Clear previous conversion results without touching queued sources",
    ),
    (
        "batch.selection",
        "Read staged batch files and output folder",
    ),
    (
        "batch.start",
        "Convert up to 200 verified 3MF sources sequentially using the selected OEM",
    ),
    (
        "batch.status",
        "Read live batch progress and per-file receipts",
    ),
    (
        "batch.cancel",
        "Cancel pending batch work and stop current Rust conversion worker",
    ),
    (
        "app.status",
        "Read source, OEM selection, conversion job and result",
    ),
    (
        "source.inspect",
        "Inspect 3MF source without changing any file",
    ),
    ("source.select", "Select and inspect a 3MF source"),
    (
        "profiles.discover",
        "List locally installed Rust-resolved OEM targets",
    ),
    (
        "profiles.select",
        "Select and verify an installed OEM printer for source",
    ),
    (
        "profiles.materials",
        "List exact matching installed OEM material presets",
    ),
    (
        "bundle.load",
        "Validate a complete, pre-resolved OEM bundle",
    ),
    (
        "printer.select",
        "Select the printer from the validated OEM bundle",
    ),
    (
        "material.select",
        "Select a verified installed OEM preset for one material type",
    ),
    (
        "conversion.start",
        "Start an existing Rust native conversion worker",
    ),
    ("conversion.status", "Read conversion job status and result"),
    (
        "conversion.cancel",
        "Cancel an active native conversion worker",
    ),
    (
        "slicer.open",
        "Open the output in an explicitly chosen slicer",
    ),
    (
        "slicer.open_selected",
        "Open converted output in selected installed OEM slicer",
    ),
    (
        "slicer.open_batch_output",
        "Open only a completed batch output recorded for the requested source and target",
    ),
];

fn arg<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing nonempty argument: {name}"))
}

fn json_error(message: impl Into<String>) -> Value {
    json!({"ok": false, "error": message.into()})
}

fn user_path(value: &Value, key: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(arg(value, key)?))
}

fn engine_path(explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(PathBuf::from(path));
    }
    let current = std::env::current_exe().map_err(|e| format!("executable path: {e}"))?;
    let filename = if cfg!(windows) {
        "slicebridge-3mf-probe.exe"
    } else {
        "slicebridge-3mf-probe"
    };
    Ok(current.with_file_name(filename))
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().expect("registry mutex");
        json!({
            "source": state.source.as_ref().map(|p| p.to_string_lossy().to_string()),
            "source_revision": state.source_revision,
            "facts": state.facts,
            "projects": state.recent_projects,
            "batch_selection": {
                "paths": state.batch_staged,
                "output_dir": state.batch_output_dir,
            },
            "bundle_path": state.bundle_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "printer": state.bundle_label,
            "target": state.target,
            "nozzle": state.nozzle,
            "installed_oem": state.installed_oem,
            "materials": state.selected_materials,
            "output": state.output.as_ref().map(|p| p.to_string_lossy().to_string()),
            "phase": if state.phase.is_empty() { "idle" } else { &state.phase },
            "result": state.last_result,
            "batch": state.batch,
        })
    }

    pub fn execute(&self, command: &str, params: &Value) -> Value {
        let result = self.dispatch(command, params);
        match result {
            Ok(value) => json!({"ok": true, "command": command, "data": value}),
            Err(error) => json_error(error),
        }
    }

    fn dispatch(&self, command: &str, params: &Value) -> Result<Value, String> {
        match command {
            "commands.list" => Ok(json!({"commands": COMMANDS.iter()
                .map(|(name, description)| json!({"name": name, "description": description}))
                .collect::<Vec<_>>()})),
            "app.status" | "conversion.status" => Ok(self.snapshot()),
            "printers.catalog" => my_printers::catalog_snapshot(),
            "printers.installed_prusa" => Ok(prusa::installed_prusa_reply()),
            "printers.installed_orca" => Ok(orca_discovery_reply(orca_profiles::discover())),
            "updates.version" => {
                let (os, arch) = updates::current_platform();
                Ok(json!({
                    "version":updates::INSTALLED_VERSION,
                    "platform":format!("{os}-{arch}")
                }))
            }
            "updates.check" => updates::check_installed(),
            "browser.setup.status" => browser_setup::status(),
            "browser.setup.store" => browser_setup::unpublished_store(arg(params, "browser")?),
            "browser.setup.page" => browser_setup::manager_page(arg(params, "browser")?),
            "browser.setup.prepare" => browser_setup::prepare(arg(params, "browser")?),
            "browser.setup.open" => browser_setup::open(
                arg(params, "browser")?,
                arg(params, "kind")?,
                params.get("mode").and_then(Value::as_str),
            ),
            "updates.page" => {
                let kind = params
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or("invalid update page")?;
                let url = match kind {
                    "releases" => updates::RELEASES_URL.to_owned(),
                    "builds" => updates::BUILDS_URL.to_owned(),
                    "release" => {
                        let tag = params
                            .get("tag")
                            .and_then(Value::as_str)
                            .ok_or("invalid release tag")?;
                        updates::release_url(tag)?
                    }
                    _ => return Err("invalid update page".into()),
                };
                Ok(json!({"url":url}))
            }
            "makerworld.origins" => {
                let paths = params
                    .get("paths")
                    .and_then(Value::as_array)
                    .ok_or("paths must be an array")?;
                if paths.len() > 200 {
                    return Err("too many paths".into());
                }
                let mut results = serde_json::Map::new();
                for path in paths {
                    let path = path
                        .as_str()
                        .ok_or("MakerWorld source path must be a string")?;
                    let source = makerworld::detect(Path::new(path));
                    if source.kind != "unknown" {
                        results.insert(path.to_owned(), source.as_value());
                    }
                }
                Ok(Value::Object(results))
            }
            "makerworld.link.preview" => {
                let raw = params
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("external link is required")?;
                makerworld::parse_link(raw).map(|link| link.as_value())
            }
            "settings.get" => printer_settings::current(),
            "settings.recover" => {
                if params.get("confirmed").and_then(Value::as_bool) != Some(true) {
                    return Err("settings recovery requires explicit confirmation".into());
                }
                let state = self.state.lock().map_err(|err| err.to_string())?;
                if busy(&state) {
                    return Err("cannot recover settings during conversion".into());
                }
                printer_settings::recover_invalid()
            }
            "settings.window.remember" => {
                let width = params
                    .get("width")
                    .and_then(Value::as_i64)
                    .ok_or("window width must be integer")?;
                let height = params
                    .get("height")
                    .and_then(Value::as_i64)
                    .ok_or("window height must be integer")?;
                let _guard = self.state.lock().map_err(|err| err.to_string())?;
                printer_settings::remember_window_size(width, height)
            }
            "settings.developer.set" => {
                let enabled = params
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("Developer Mode must be a boolean")?;
                let _guard = self.state.lock().map_err(|e| e.to_string())?;
                let verified = printer_settings::set_developer_mode(enabled)?;
                Ok(json!({"enabled":verified["developerMode"]}))
            }
            "settings.options" => {
                let target = params
                    .get("target")
                    .and_then(Value::as_str)
                    .ok_or("target is required")?;
                let nozzle = params
                    .get("nozzle")
                    .and_then(Value::as_str)
                    .unwrap_or("auto");
                let slicer = params
                    .get("slicer")
                    .and_then(Value::as_str)
                    .unwrap_or("auto");
                let material = params
                    .get("material")
                    .and_then(Value::as_str)
                    .unwrap_or("PLA");
                printer_settings::options(target, nozzle, slicer, material)
            }
            "settings.save" => {
                let target = params
                    .get("target")
                    .and_then(Value::as_str)
                    .ok_or("target is required")?;
                let preference = params
                    .get("pref")
                    .ok_or("printer preferences are required")?;
                let mut state = self.state.lock().map_err(|err| err.to_string())?;
                if busy(&state) {
                    return Err("cannot modify printer settings during conversion".into());
                }
                let persisted = printer_settings::save(target, preference)?;
                invalidate_active_settings_target(&mut state, target);
                Ok(persisted)
            }
            "printers.get" => my_printers::read(
                &my_printers::experimental_path()?,
                &my_printers::available_installed_ids(),
            ),
            "printers.save" => {
                let ids = params
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or("missing printer ID list")?
                    .iter()
                    .map(|id| {
                        id.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "invalid printer ID".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let state = self.state.lock().map_err(|err| err.to_string())?;
                if busy(&state) {
                    return Err("cannot change saved printers during conversion".into());
                }
                // Serialize writes through this shared Registry to avoid lost
                // updates across GUI, CLI and local attach calls.
                my_printers::save(
                    &my_printers::experimental_path()?,
                    &ids,
                    &my_printers::available_installed_ids(),
                )
            }
            "projects.list" => {
                let state = self.state.lock().map_err(|e| e.to_string())?;
                Ok(json!({"paths":state.recent_projects,
                    "active":state.source.as_ref().map(|p|p.to_string_lossy().to_string())}))
            }
            "projects.forget" => {
                let path = user_path(params, "path")?;
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                let before = state.recent_projects.len();
                state
                    .recent_projects
                    .retain(|entry| !same_path(entry, &path));
                Ok(json!({"removed":before != state.recent_projects.len(),
                    "remaining":state.recent_projects.len()}))
            }
            "projects.clear" => {
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                let removed = state.recent_projects.len();
                state.recent_projects.clear();
                Ok(json!({"removed":removed}))
            }
            "batch.stage" => self.batch_stage(params),
            "batch.clear" => self.batch_clear(),
            "batch.remove" => self.batch_remove(params),
            "results.clear" => {
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if busy(&state) {
                    return Err("cannot clear results during conversion".into());
                }
                clear_conversion_results(&mut state);
                Ok(json!({"cleared":true}))
            }
            "batch.directory" => self.batch_directory(params),
            "batch.selection" => self.batch_selection(),
            "batch.start" => self.batch_start(params),
            "batch.status" => {
                let state = self.state.lock().map_err(|e| e.to_string())?;
                Ok(state
                    .batch
                    .clone()
                    .unwrap_or_else(|| json!({"phase":"idle","total":0,"completed":0,"items":[]})))
            }
            "batch.cancel" => self.batch_cancel(),
            "source.inspect" | "source.select" => {
                let path = user_path(params, "path")?;
                let facts = inspect_package(&path)?;
                let nozzle = inspect_source_nozzle(&path)?;
                let receipt = json!({
                    "path": path.to_string_lossy(),
                    "vertices": facts.vertices,
                    "triangles": facts.triangles,
                    "objects": facts.objects,
                    "instances": facts.instances,
                    "slice_caches": facts.slice_caches,
                    "printer_model": facts.printer_model,
                    "source_nozzle": nozzle["source_nozzle"],
                    "source_nozzle_source": nozzle["source_nozzle_source"],
                    "source_nozzle_assumed": nozzle["source_nozzle_assumed"],
                });
                if command == "source.select" {
                    let mut state = self.state.lock().map_err(|e| e.to_string())?;
                    if busy(&state) {
                        return Err("cannot change source during conversion".into());
                    }
                    state
                        .recent_projects
                        .retain(|entry| !same_path(entry, &path));
                    state.recent_projects.insert(0, path.clone());
                    state.recent_projects.truncate(MAX_RECENT_PROJECTS);
                    state.source = Some(path);
                    state.source_revision = state.source_revision.wrapping_add(1);
                    state.facts = Some(receipt.clone());
                    state.last_result = None;
                    state.output = None;
                    state.phase = "idle".into();
                    state.bundle_path = None;
                    state.bundle_label = None;
                    state.bundle = None;
                    state.installed_oem = false;
                    state.target = None;
                    state.nozzle = None;
                    state.selected_materials.clear();
                }
                Ok(receipt)
            }
            "profiles.discover" => Ok(oem::discover()),
            "profiles.select" => {
                let target = arg(params, "target")?.to_string();
                let requested_nozzle = params
                    .get("nozzle")
                    .and_then(Value::as_str)
                    .unwrap_or("auto")
                    .to_string();
                let source = {
                    let state = self.state.lock().map_err(|e| e.to_string())?;
                    if busy(&state) {
                        return Err("cannot select OEM while converting".into());
                    }
                    state
                        .source
                        .clone()
                        .ok_or("select source 3MF before OEM profile")?
                };
                // Auto follows the source 3MF nozzle, unless a fixed physical
                // nozzle was saved for the target. No conversion happens here.
                let source_nozzle = inspect_source_nozzle(&source)?;
                let source_diameter = source_nozzle["source_nozzle"]
                    .as_str()
                    .ok_or("source 3MF nozzle unavailable")?;
                let saved = printer_settings::current()?;
                let (nozzle, overrides) = printer_settings::profile_choice_for_source(
                    &saved,
                    &target,
                    &requested_nozzle,
                    source_diameter,
                )?;
                let slicer = saved["printers"][&target]["slicer"]
                    .as_str()
                    .unwrap_or("auto");
                let bundle = oem::resolve_configured(&target, &nozzle, slicer, &overrides)?;
                let verified = validate_native_autonomous_input(&bundle, &source)?;
                let label = verified["printer"]
                    .as_str()
                    .ok_or("OEM model missing")?
                    .to_owned();
                let selected_materials = bundle["profile"]["filament_profiles"]
                    .as_object()
                    .ok_or("installed material profile catalog missing")?
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|name| (k.clone(), name.to_string())))
                    .collect();
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if busy(&state) || state.source.as_ref() != Some(&source) {
                    return Err("source or job changed while verifying installed profile".into());
                }
                // A Settings save may have invalidated this OEM while disk profiles
                // were being resolved. Recheck before committing the verified bundle.
                if printer_settings::current()? != saved {
                    return Err("printer settings changed during OEM profile verification".into());
                }
                state.bundle_path = None;
                state.bundle_label = Some(label.clone());
                state.bundle = Some(bundle);
                state.installed_oem = true;
                state.target = Some(target);
                state.nozzle = Some(nozzle);
                state.selected_materials = selected_materials;
                state.output = None;
                state.last_result = None;
                state.phase = "idle".into();
                Ok(json!({"printer":label,"validation":verified,
                    "materials":state.selected_materials,"installed_oem":true}))
            }
            "profiles.materials" => {
                let material = arg(params, "material")?;
                let state = self.state.lock().map_err(|e| e.to_string())?;
                if !state.installed_oem {
                    return Err("select installed OEM printer first".into());
                }
                let target = state.target.as_deref().ok_or("missing installed target")?;
                let nozzle = state.nozzle.as_deref().ok_or("missing installed nozzle")?;
                let choices = oem::choices(target, nozzle, material)?;
                Ok(json!({"material":material,"presets":choices,
                    "selected":state.selected_materials.get(material)}))
            }
            "bundle.load" => {
                let path = user_path(params, "path")?;
                let bytes = fs::read(&path).map_err(|e| format!("cannot read OEM bundle: {e}"))?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("OEM bundle JSON exceeds 8 MB".into());
                }
                let bundle: Value = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid OEM bundle JSON: {e}"))?;
                let source = self
                    .state
                    .lock()
                    .map_err(|e| e.to_string())?
                    .source
                    .clone()
                    .ok_or("select a source 3MF before loading an OEM bundle")?;
                // Use the engine's real source-owned palette, not an invented
                // material selection or a copy of the conversion planner.
                let verified = validate_native_autonomous_input(&bundle, &source)?;
                let label = verified["printer"]
                    .as_str()
                    .unwrap_or("Verified OEM profile")
                    .to_string();
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if busy(&state) {
                    return Err("cannot change OEM bundle during conversion".into());
                }
                if state.source.as_ref() != Some(&source) {
                    return Err("source changed during OEM verification".into());
                }
                state.bundle_path = Some(path);
                state.bundle_label = Some(label.clone());
                state.target = bundle["target"].as_str().map(str::to_owned);
                state.nozzle = bundle["nozzle"].as_str().map(str::to_owned);
                state.bundle = Some(bundle);
                state.installed_oem = false;
                state.selected_materials.clear();
                state.output = None;
                state.last_result = None;
                state.phase = "idle".into();
                Ok(json!({"printer": label, "validation": verified, "installed_oem":false}))
            }
            "printer.select" => {
                let target = arg(params, "name")?;
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                let existing = state
                    .bundle_label
                    .as_deref()
                    .ok_or("load a verified OEM bundle before selecting a printer")?;
                if target != existing {
                    return Err("printer selection must match the verified OEM bundle".into());
                }
                state.bundle_label = Some(target.to_string());
                Ok(json!({"printer": target}))
            }
            "material.select" => {
                let material = arg(params, "material")?.to_string();
                let preset = arg(params, "preset")?.to_string();
                let (target, nozzle, source, mut overrides, slicer_id) = {
                    let state = self.state.lock().map_err(|e| e.to_string())?;
                    if busy(&state) {
                        return Err("cannot change materials while converting".into());
                    }
                    if !state.installed_oem {
                        return Err(
                            "select a locally installed OEM printer before material selection"
                                .into(),
                        );
                    }
                    (
                        state.target.clone().ok_or("no installed target")?,
                        state.nozzle.clone().ok_or("no installed nozzle")?,
                        state.source.clone().ok_or("no selected source")?,
                        state.selected_materials.clone(),
                        state
                            .bundle
                            .as_ref()
                            .and_then(|bundle| bundle["slicer_id"].as_str())
                            .ok_or("installed source slicer missing")?
                            .to_owned(),
                    )
                };
                overrides.insert(material.clone(), preset.clone());
                // Rebuild from the SAME installed slicer that owns the selected
                // OEM. Another vendor/fork may expose a different preset leaf.
                let bundle = oem::resolve_configured(&target, &nozzle, &slicer_id, &overrides)?;
                let verified = validate_native_autonomous_input(&bundle, &source)?;
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if busy(&state)
                    || state.source.as_ref() != Some(&source)
                    || state.target.as_deref() != Some(&target)
                    || state.nozzle.as_deref() != Some(&nozzle)
                {
                    return Err("source or printer changed during material verification".into());
                }
                state.bundle = Some(bundle.clone());
                state.selected_materials = overrides;
                state.output = None;
                state.last_result = None;
                state.phase = "idle".into();
                Ok(json!({"material":material,"preset":preset,"applied":true,
                    "temperature":bundle["material_settings"][&material]["nozzle_temperature"],
                    "flow_ratio":bundle["material_settings"][&material]["filament_flow_ratio"],
                    "validation":verified}))
            }
            "conversion.start" => self.start_conversion(params),
            "conversion.cancel" => {
                let state = self.state.lock().map_err(|e| e.to_string())?;
                if state.phase != "running" {
                    return Ok(json!({"cancel_requested": false, "phase": state.phase}));
                }
                if let Some(cancel) = &state.cancel {
                    cancel.store(true, Ordering::SeqCst);
                }
                Ok(json!({"cancel_requested": true, "phase": "cancelling"}))
            }
            "slicer.open" => {
                let slicer = user_path(params, "executable")?;
                let output = if let Some(output) = params.get("path").and_then(Value::as_str) {
                    PathBuf::from(output)
                } else {
                    self.state
                        .lock()
                        .map_err(|e| e.to_string())?
                        .output
                        .clone()
                        .ok_or("no converted output available")?
                };
                if !output.is_file() {
                    return Err("converted output does not exist".into());
                }
                validate_recorded_3mf(&output)?;
                if !slicer.is_file() {
                    return Err("slicer executable does not exist".into());
                }
                launch_slicer(&slicer, &output)?;
                Ok(json!({"launched": true, "slicer": slicer.to_string_lossy(),
                    "file": output.to_string_lossy(), "slicing": false}))
            }
            "slicer.open_selected" => {
                let state = self.state.lock().map_err(|e| e.to_string())?;
                if state.phase != "completed" {
                    return Err("convert source before opening slicer".into());
                }
                if !state.installed_oem {
                    return Err("select an installed OEM printer before opening its slicer".into());
                }
                let target = state.target.as_deref().ok_or("no selected OEM target")?;
                let nozzle = state.nozzle.as_deref().ok_or("no selected OEM nozzle")?;
                let output = state
                    .output
                    .clone()
                    .ok_or("convert source before opening slicer")?;
                if !output.is_file() {
                    return Err("converted output is missing".into());
                }
                validate_recorded_3mf(&output)?;
                let slicer_id = state
                    .bundle
                    .as_ref()
                    .and_then(|bundle| bundle["slicer_id"].as_str())
                    .ok_or("selected OEM slicer identity missing")?;
                let slicer = oem::installed_executable_configured(target, nozzle, slicer_id)?;
                drop(state);
                launch_slicer(&slicer, &output)?;
                Ok(json!({"launched":true,"slicer":slicer.to_string_lossy(),
                    "file":output.to_string_lossy(),"slicing":false}))
            }
            "slicer.open_batch_output" => {
                let source = arg(params, "source")?;
                let target = arg(params, "target")?;
                let recorded = {
                    let state = self.state.lock().map_err(|e| e.to_string())?;
                    let batch = state.batch.as_ref().ok_or("no batch output available")?;
                    recorded_batch_output(batch, source, target)?
                };
                validate_recorded_3mf(&recorded.path)?;
                let slicer = oem::installed_executable_configured(
                    target,
                    &recorded.target_nozzle,
                    &recorded.slicer_id,
                )?;
                // The output path comes exclusively from this registry's completed
                // receipt; the caller cannot substitute another file or executable.
                launch_slicer(&slicer, &recorded.path)?;
                Ok(json!({
                    "launched":true,
                    "slicer":slicer.to_string_lossy(),
                    "file":recorded.path.to_string_lossy(),
                    "slicing":false
                }))
            }
            _ => Err(format!("unknown command: {command}")),
        }
    }

    fn start_conversion(&self, params: &Value) -> Result<Value, String> {
        let output = user_path(params, "output")?;
        if output.exists() {
            return Err("destination exists; will not overwrite".into());
        }
        let engine = engine_path(params.get("engine").and_then(Value::as_str))?;
        if !engine.is_file() {
            return Err(format!(
                "Rust conversion worker not found: {}",
                engine.display()
            ));
        }
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        if busy(&state) {
            return Err("conversion is already running".into());
        }
        // Optional GUI-only lease: an external source.select to the same path
        // must not convert using a destination from the previous selection.
        if let Some(expected) = params.get("expected_source_revision") {
            if expected.as_u64() != Some(state.source_revision) {
                return Err("source changed since destination was chosen".into());
            }
        }
        let source = state.source.clone().ok_or("select a source 3MF first")?;
        let bundle = state
            .bundle
            .clone()
            .ok_or("select a verified OEM bundle first")?;
        if same_path(&source, &output) {
            return Err("output must differ from source".into());
        }
        // Only the verified, source-owned OEM snapshot is passed to the Rust
        // conversion worker. An ephemeral file preserves the existing worker
        // interface without changing the real vendor installation.
        let tmp = std::env::temp_dir().join(format!(
            "slicebridge-verified-oem-{}-{}.json",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("create temporary OEM snapshot: {e}"))?;
        if let Err(e) = file.write_all(bundle.to_string().as_bytes()) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("write verified OEM snapshot: {e}"));
        }
        drop(file);
        let child = match Command::new(&engine)
            .arg("--convert-native-autonomous")
            .arg(&tmp)
            .arg(&source)
            .arg(&output)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(format!("Rust engine start failed: {e}"));
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel = Some(cancel.clone());
        state.phase = "running".into();
        state.output = None;
        state.last_result = None;
        drop(state);

        let shared = self.state.clone();
        let output_for_receipt = output.clone();
        thread::spawn(move || {
            let mut child = child;
            let (phase, receipt) = loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let mut stdout = String::new();
                        if let Some(mut pipe) = child.stdout.take() {
                            let _ = pipe.read_to_string(&mut stdout);
                        }
                        let mut stderr = String::new();
                        if let Some(mut pipe) = child.stderr.take() {
                            let _ = pipe.read_to_string(&mut stderr);
                        }
                        let parsed = serde_json::from_str::<Value>(stdout.trim())
                            .unwrap_or(json!({"stdout": stdout.trim()}));
                        if status.success()
                            && parsed.get("ok") == Some(&Value::Bool(true))
                            && output_for_receipt.is_file()
                        {
                            break (
                                "completed",
                                json!({"engine": parsed, "file": output_for_receipt}),
                            );
                        }
                        break (
                            "failed",
                            json!({"exit_code": status.code(),
                            "engine": parsed, "stderr": stderr}),
                        );
                    }
                    Ok(None) => {
                        if cancel.load(Ordering::SeqCst) {
                            let _ = child.kill();
                            let _ = child.wait();
                            break ("cancelled", json!({"cancelled": true}));
                        }
                        thread::sleep(Duration::from_millis(80));
                    }
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break ("failed", json!({"error": format!("worker wait: {e}")}));
                    }
                }
            };
            let _ = fs::remove_file(&tmp);
            if let Ok(mut state) = shared.lock() {
                state.phase = phase.into();
                state.last_result = Some(receipt);
                state.cancel = None;
                state.output = if phase == "completed" {
                    Some(output_for_receipt)
                } else {
                    None
                };
            }
        });
        Ok(json!({"started": true, "phase": "running", "output": output.to_string_lossy()}))
    }
}

fn slicer_command(executable: &Path, output: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        // Match Go's exec.Command("open", "-a", app, output): open the 3MF
        // document in the selected app, rather than passing it as app arguments.
        let mut command = Command::new("open");
        command.arg("-a").arg(executable).arg(output);
        command
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut command = Command::new(executable);
        command.arg(output);
        command
    }
}

fn launch_slicer(executable: &Path, output: &Path) -> Result<(), String> {
    slicer_command(executable, output)
        .spawn()
        .map_err(|error| format!("could not open selected slicer: {error}"))?;
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_reply_matches_go_inventory_shape_without_conversion_or_private_paths() {
        let reply = orca_discovery_reply(orca_profiles::OrcaDiscovery {
            targets: vec![orca_profiles::OrcaTarget {
                id: "orca-snapmaker-u1-0123456789".into(),
                title: "U1 0.4".into(),
                slicer: "Snapmaker Orca".into(),
                vendor: "Snapmaker".into(),
                machine: "U1 0.4 nozzle".into(),
                model: "U1".into(),
                nozzle: "0.4".into(),
                physical_tools: 4,
                materials: vec!["PLA".into()],
            }],
            rejected: vec![orca_profiles::OrcaRejection {
                vendor: "Other".into(),
                machine: "broken profile".into(),
                slicer: "OrcaSlicer".into(),
                reason: "missing compatible PLA".into(),
            }],
        });
        assert_eq!(reply["targets"].as_array().unwrap().len(), 1);
        assert_eq!(reply["targets"][0]["ready"], false);
        assert_eq!(reply["targets"][0]["id"], "orca-snapmaker-u1-0123456789");
        assert_eq!(reply["targets"][0]["physicalTools"], 4);
        assert_eq!(reply["targets"][0]["materials"], json!(["PLA"]));
        assert_eq!(reply["targets"][0]["reason"], ORCA_EXPORT_NOT_VERIFIED);
        assert!(reply["targets"][0].get("bundle").is_none());
        assert!(reply["targets"][0].get("root").is_none());
        assert!(reply["targets"][0].get("executable").is_none());
        assert_eq!(reply["rejected"][0]["reason"], "missing compatible PLA");
    }

    #[test]
    fn orca_registry_is_read_only_and_conversion_unverified() {
        let registry = Registry::new();
        let before = registry.snapshot();
        let result = registry.execute("printers.installed_orca", &json!({}));
        assert_eq!(result["ok"], true);
        let data = &result["data"];
        assert!(data["targets"].is_array());
        assert!(data["rejected"].is_array());
        for target in data["targets"].as_array().unwrap() {
            assert_eq!(target["ready"], false);
            assert_eq!(target["reason"], ORCA_EXPORT_NOT_VERIFIED);
            assert!(target.get("root").is_none());
            assert!(target.get("bundle").is_none());
        }
        assert_eq!(registry.snapshot(), before);
    }

    #[test]
    fn prusa_discovery_is_read_only_and_conversion_unverified() {
        let registry = Registry::new();
        let before = registry.snapshot();
        let result = registry.execute("printers.installed_prusa", &json!({}));
        assert_eq!(result["ok"], true);
        let data = &result["data"];
        assert!(data["targets"].is_array());
        assert!(data["rejected"].is_array());
        for target in data["targets"].as_array().unwrap() {
            assert_eq!(target["ready"], false);
            assert!(target["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()));
        }
        assert_eq!(registry.snapshot(), before);
    }

    #[test]
    fn registry_commands_are_discoverable() {
        let registry = Registry::new();
        let result = registry.execute("commands.list", &json!({}));
        assert_eq!(result["ok"], true);
        assert_eq!(
            result["data"]["commands"].as_array().unwrap().len(),
            COMMANDS.len()
        );
    }

    #[test]
    fn update_version_and_allowlisted_pages_have_no_side_effects() {
        let registry = Registry::new();
        let baseline = registry.snapshot();
        let version = registry.execute("updates.version", &json!({}));
        assert_eq!(version["ok"], true);
        assert_eq!(version["data"]["version"], updates::INSTALLED_VERSION);
        let releases = registry.execute("updates.page", &json!({"kind":"releases"}));
        assert_eq!(releases["data"]["url"], updates::RELEASES_URL);
        let builds = registry.execute("updates.page", &json!({"kind":"builds"}));
        assert_eq!(builds["data"]["url"], updates::BUILDS_URL);
        let release = registry.execute(
            "updates.page",
            &json!({
                "kind":"release","tag":"v0.4.0-beta.7"
            }),
        );
        assert_eq!(
            release["data"]["url"],
            "https://github.com/nocturney/slicebridge/releases/tag/v0.4.0-beta.7"
        );
        for params in [
            json!({}),
            json!({"kind":"release"}),
            json!({"kind":"release","tag":"v0.4.0-beta.7/../../evil"}),
            json!({"kind":"https://evil.test"}),
            json!({"kind":"source"}),
            json!({"kind":"builds/../../"}),
        ] {
            assert_eq!(registry.execute("updates.page", &params)["ok"], false);
        }
        assert_eq!(registry.snapshot(), baseline);
    }

    #[test]
    fn makerworld_preview_is_read_only_and_refuses_untrusted_links() {
        let registry = Registry::new();
        let before = registry.snapshot();
        let valid = registry.execute(
            "makerworld.link.preview",
            &json!({
                "url":"slicebridge://import?url=https%3A%2F%2Fmakerworld.com%2Fen%2Fmodels%2F123"
            }),
        );
        assert_eq!(valid["ok"], true);
        assert_eq!(valid["data"]["scheme"], "slicebridge");
        assert_eq!(valid["data"]["source"]["kind"], "makerworld");
        assert_eq!(valid["data"]["source"]["confidence"], "high");
        for input in [
            json!({}),
            json!({"url":"https://makerworld.com/a.3mf"}),
            json!({"url":"bambustudioopen://https%3A%2F%2Fmakerworld.com.evil.test%2Fa.3mf"}),
            json!({"url":"slicebridge://import?url=file%3A%2F%2Fetc%2Fpasswd"}),
        ] {
            assert_eq!(
                registry.execute("makerworld.link.preview", &input)["ok"],
                false
            );
        }
        assert_eq!(registry.snapshot(), before);
    }

    #[test]
    fn makerworld_provenance_does_not_modify_local_model_or_accept_oversize_list() {
        let registry = Registry::new();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-one-color.3mf");
        let original = fs::read(&source).unwrap();
        let prior = registry.snapshot();
        let reply = registry.execute("makerworld.origins", &json!({"paths":[source]}));
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(reply["data"].as_object().unwrap().is_empty());
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(registry.snapshot(), prior);
        assert_eq!(
            registry.execute("makerworld.origins", &json!({"paths":[]}))["data"],
            json!({})
        );
        assert_eq!(
            registry.execute("makerworld.origins", &json!({"paths":"not-array"}))["ok"],
            false
        );
        assert_eq!(
            registry.execute("makerworld.origins", &json!({"paths":[12]}))["ok"],
            false
        );
        let oversized = vec!["missing.3mf"; 201];
        assert_eq!(
            registry.execute("makerworld.origins", &json!({"paths":oversized}))["ok"],
            false
        );
    }

    #[test]
    fn developer_mode_set_requires_boolean_before_any_settings_write() {
        let registry = Registry::new();
        for params in [json!({}), json!({"enabled":"true"}), json!({"enabled":1})] {
            let result = registry.execute("settings.developer.set", &params);
            assert_eq!(result["ok"], false);
            assert!(result["error"].as_str().unwrap_or("").contains("boolean"));
        }
        assert_eq!(registry.snapshot()["phase"], "idle");
    }

    #[test]
    fn nonexistent_source_does_not_change_state() {
        let registry = Registry::new();
        let result = registry.execute("source.select", &json!({"path": "nonexistent.3mf"}));
        assert_eq!(result["ok"], false);
        assert_eq!(registry.snapshot()["source"], Value::Null);
    }

    #[test]
    fn source_inspection_and_selection_expose_original_nozzle_provenance() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-nozzle-0.6.3mf");
        let registry = Registry::new();
        let read = registry.execute("source.inspect", &json!({"path":source}));
        assert_eq!(read["ok"], true, "{read}");
        assert_eq!(read["data"]["source_nozzle"], "0.6");
        assert_eq!(read["data"]["source_nozzle_source"], "nozzle_diameter");
        assert_eq!(read["data"]["source_nozzle_assumed"], false);
        // Go sourceSummary exposes model object and build instance counts.
        // Registry inspect/select must preserve the Rust inspector's real facts.
        let source_facts = slicebridge_3mf_probe::inspect_package(&source).unwrap();
        assert_eq!(read["data"]["objects"], source_facts.objects);
        assert_eq!(read["data"]["instances"], source_facts.instances);
        assert!(source_facts.objects > 0);
        assert!(registry.snapshot()["source"].is_null());
        assert_eq!(registry.snapshot()["source_revision"], 0);
        let selected = registry.execute("source.select", &json!({"path":source}));
        assert_eq!(selected["ok"], true, "{selected}");
        assert_eq!(registry.snapshot()["facts"]["source_nozzle"], "0.6");
        assert_eq!(registry.snapshot()["source_revision"], 1);
        // This is a real path-validating Registry operation, not a mock.
        let again = registry.execute("source.select", &json!({"path":source}));
        assert_eq!(again["ok"], true, "{again}");
        assert_eq!(registry.snapshot()["source_revision"], 2);
        let assumed =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-one-color.3mf");
        let legacy = registry.execute("source.inspect", &json!({"path":assumed}));
        assert_eq!(legacy["ok"], true, "{legacy}");
        assert_eq!(legacy["data"]["source_nozzle"], "0.4");
        assert_eq!(legacy["data"]["source_nozzle_assumed"], true);
    }

    #[test]
    fn conversion_requires_verified_bundle() {
        let registry = Registry::new();
        let result = registry.execute("conversion.start", &json!({"output": "out.3mf"}));
        assert_eq!(result["ok"], false);
        assert_eq!(registry.snapshot()["phase"], "idle");
    }

    #[test]
    fn conversion_with_old_gui_revision_refuses_before_worker_launch() {
        let registry = Registry::new();
        let worker = std::env::current_exe().expect("test executable path");
        let output = std::env::temp_dir().join(format!(
            "slicebridge-source-revision-rejection-{}.3mf",
            std::process::id()
        ));
        assert!(!output.exists(), "test must not touch an existing output");
        let reply = registry.execute(
            "conversion.start",
            &json!({"output":output, "engine":worker, "expected_source_revision":7}),
        );
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"]
            .as_str()
            .unwrap_or("")
            .contains("source changed since destination"));
        assert!(!output.exists());
        assert_eq!(registry.snapshot()["phase"], "idle");
    }

    #[test]
    fn saving_active_printer_settings_invalidates_old_oem_without_deleting_results() {
        let mut state = Session::default();
        state.target = Some("u1".into());
        state.nozzle = Some("0.4".into());
        state.installed_oem = true;
        state.bundle = Some(json!({"verified":true}));
        state.bundle_label = Some("Snapmaker U1".into());
        state
            .selected_materials
            .insert("PLA".into(), "Old PLA".into());
        state.output = Some("previous-result.3mf".into());
        invalidate_active_settings_target(&mut state, "qidi");
        assert_eq!(state.target.as_deref(), Some("u1"));
        assert!(state.installed_oem);
        invalidate_active_settings_target(&mut state, "u1");
        assert!(state.target.is_none() && state.nozzle.is_none());
        assert!(state.bundle.is_none() && state.bundle_label.is_none());
        assert!(!state.installed_oem && state.selected_materials.is_empty());
        assert_eq!(
            state.output.as_deref(),
            Some(Path::new("previous-result.3mf"))
        );
    }

    #[test]
    fn changing_source_invalidates_old_oem_and_output() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-one-color.3mf");
        assert!(
            fixture.is_file(),
            "permissioned repository source fixture missing"
        );
        let registry = Registry::new();
        {
            let mut old = registry.state.lock().unwrap();
            old.phase = "completed".into();
            old.source = Some("stale.3mf".into());
            old.bundle = Some(json!({"target":"u1"}));
            old.bundle_path = Some("stale-oem.json".into());
            old.bundle_label = Some("Snapmaker U1".into());
            old.installed_oem = true;
            old.target = Some("u1".into());
            old.nozzle = Some("0.4".into());
            old.output = Some("old-output.3mf".into());
        }
        let reply = registry.execute("source.select", &json!({"path": fixture}));
        assert_eq!(reply["ok"], true, "{reply}");
        let status = registry.snapshot();
        assert_eq!(status["phase"], "idle");
        assert!(status["output"].is_null());
        assert!(status["bundle_path"].is_null());
        assert!(status["printer"].is_null());
        assert!(status["target"].is_null());
        assert_eq!(status["installed_oem"], false);
    }

    #[test]
    fn material_change_never_accepts_display_only_values() {
        let registry = Registry::new();
        let result = registry.execute(
            "material.select",
            &json!({"material":"PETG","preset":"Fictional PETG"}),
        );
        assert_eq!(result["ok"], false);
        assert!(registry.snapshot()["materials"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recorded_batch_output_requires_exact_completed_source_and_target() {
        let single = json!({
            "target":"u1",
            "items":[{
                "source":"/sources/a.3mf",
                "phase":"completed",
                "receipt":{
                    "output":"/sources/U1-a.3mf",
                    "target_nozzle":"0.4",
                    "slicer_id":"snapmaker-orca"
                }
            }]
        });
        let output = recorded_batch_output(&single, "/sources/a.3mf", "u1").unwrap();
        assert_eq!(output.path, PathBuf::from("/sources/U1-a.3mf"));
        assert_eq!(output.target_nozzle, "0.4");
        assert_eq!(output.slicer_id, "snapmaker-orca");
        assert!(recorded_batch_output(&single, "/sources/a.3mf", "qidi").is_err());
        assert!(recorded_batch_output(&single, "/sources/other.3mf", "u1").is_err());

        let multi = json!({"items":[{
            "source":"/sources/b.3mf",
            "targets":[
                {"target":"c5","phase":"failed","receipt":{"error":"bad source"}},
                {"target":"cc2","phase":"completed","receipt":{
                    "output":"/sources/CentauriCarbon2-b.3mf",
                    "target_nozzle":"0.6",
                    "slicer_id":"elegoo-slicer"
                }}
            ]
        }]});
        let output = recorded_batch_output(&multi, "/sources/b.3mf", "cc2").unwrap();
        assert_eq!(output.target_nozzle, "0.6");
        assert_eq!(output.slicer_id, "elegoo-slicer");
        assert!(recorded_batch_output(&multi, "/sources/b.3mf", "c5").is_err());
        assert!(recorded_batch_output(&multi, "/sources/b.3mf", "u1").is_err());

        let unfinished = json!({"target":"u1","items":[{
            "source":"/sources/a.3mf","phase":"running",
            "receipt":{"output":"/sources/U1-a.3mf","target_nozzle":"0.4",
                "slicer_id":"snapmaker-orca"}
        }]});
        assert!(recorded_batch_output(&unfinished, "/sources/a.3mf", "u1").is_err());
    }

    #[test]
    fn recorded_batch_output_requires_slicer_identity_and_verified_3mf_file() {
        let missing_slicer = json!({"target":"u1","items":[{
            "source":"/sources/a.3mf","phase":"completed",
            "receipt":{"output":"/sources/U1-a.3mf","target_nozzle":"0.4"}
        }]});
        assert!(recorded_batch_output(&missing_slicer, "/sources/a.3mf", "u1").is_err());

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-one-color.3mf");
        validate_recorded_3mf(&fixture).unwrap();
        assert!(validate_recorded_3mf(Path::new("/tmp/not-a-3mf.txt")).is_err());
        assert!(validate_recorded_3mf(Path::new("relative.3mf")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_slicer_open_passes_the_3mf_as_a_document_not_app_arguments() {
        let command = slicer_command(
            Path::new("/Applications/Example.app"),
            Path::new("/tmp/Example.3mf"),
        );
        assert_eq!(command.get_program(), "open");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec!["-a", "/Applications/Example.app", "/tmp/Example.3mf"]
        );
    }

    #[test]
    fn opening_selected_slicer_requires_completed_conversion() {
        let registry = Registry::new();
        let result = registry.execute("slicer.open_selected", &json!({}));
        assert_eq!(result["ok"], false);
        let batch = registry.execute(
            "slicer.open_batch_output",
            &json!({
                "source":"/sources/a.3mf","target":"u1"
            }),
        );
        assert_eq!(batch["ok"], false);
    }

    #[test]
    fn opening_selected_slicer_rejects_ineligible_output_before_launch() {
        let registry = Registry::new();
        let non_3mf = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let empty = std::env::temp_dir().join(format!(
            "slicebridge-empty-output-{}-{}.3mf",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&empty)
            .unwrap();
        for (path, expected) in [
            (non_3mf, "choose a generated .3mf file"),
            (
                empty.clone(),
                "generated output must be a regular .3mf file up to 250 MB",
            ),
        ] {
            {
                let mut state = registry.state.lock().unwrap();
                state.phase = "completed".into();
                state.installed_oem = true;
                state.target = Some("qidi".into());
                state.nozzle = Some("0.4".into());
                state.output = Some(path);
            }
            let result = registry.execute("slicer.open_selected", &json!({}));
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], expected);
        }
        fs::remove_file(empty).unwrap();
    }

    #[test]
    fn opening_explicit_slicer_rejects_ineligible_output_before_launch() {
        let registry = Registry::new();
        let non_3mf = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let empty = std::env::temp_dir().join(format!(
            "slicebridge-explicit-empty-output-{}-{}.3mf",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&empty)
            .unwrap();
        for (path, expected) in [
            (non_3mf, "choose a generated .3mf file"),
            (
                empty.clone(),
                "generated output must be a regular .3mf file up to 250 MB",
            ),
        ] {
            let result = registry.execute(
                "slicer.open",
                &json!({"executable": empty.with_extension("missing"), "path": path}),
            );
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], expected);
            registry.state.lock().unwrap().output = Some(path);
            let result = registry.execute(
                "slicer.open",
                &json!({"executable": empty.with_extension("missing")}),
            );
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], expected);
        }
        fs::remove_file(empty).unwrap();
    }

    #[test]
    fn recent_project_history_is_agent_visible_and_does_not_touch_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let one = root.join("bambu-one-color.3mf");
        let five = root.join("bambu-five-colors.3mf");
        let registry = Registry::new();
        for path in [&one, &five, &one] {
            assert_eq!(
                registry.execute("source.select", &json!({"path":path}))["ok"],
                true
            );
        }
        let history = registry.execute("projects.list", &json!({}));
        assert_eq!(history["data"]["paths"].as_array().unwrap().len(), 2);
        assert_eq!(history["data"]["paths"][0], json!(one));
        assert_eq!(registry.snapshot()["projects"][0], json!(one));
        assert_eq!(
            registry.execute("projects.forget", &json!({"path":five}))["data"]["removed"],
            true
        );
        assert_eq!(registry.snapshot()["projects"].as_array().unwrap().len(), 1);
        assert_eq!(
            registry.execute("projects.clear", &json!({}))["data"]["removed"],
            1
        );
        assert!(registry.snapshot()["projects"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(registry.snapshot()["source"], json!(one));
        assert!(one.is_file());
        assert!(five.is_file());
    }

    #[test]
    fn settings_recovery_command_requires_explicit_boolean_confirmation() {
        let registry = Registry::new();
        for args in [
            json!({}),
            json!({"confirmed":false}),
            json!({"confirmed":"true"}),
        ] {
            let reply = registry.execute("settings.recover", &args);
            assert_eq!(reply["ok"], false);
            assert!(reply["error"]
                .as_str()
                .unwrap()
                .contains("explicit confirmation"));
        }
    }

    #[test]
    fn unknown_command_rejected() {
        let result = Registry::new().execute("printer.print", &json!({}));
        assert_eq!(result["ok"], false);
    }
}

#[cfg(test)]
mod original_result_parity_tests {
    use super::*;

    #[test]
    fn clear_results_preserves_queued_source_and_printer_state() {
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.source = Some(PathBuf::from("kept-source.3mf"));
            state.batch_staged = vec![PathBuf::from("kept-source.3mf")];
            state.target = Some("qidi".into());
            state.phase = "completed".into();
            state.last_result = Some(json!({"success":true}));
            state.output = Some(PathBuf::from("kept-output.3mf"));
            state.installed_oem = true;
            state.batch = Some(json!({"phase":"completed","total":1}));
        }
        let result = registry.execute("results.clear", &json!({}));
        assert_eq!(result["ok"], true);
        let state = registry.snapshot();
        assert_eq!(state["source"], "kept-source.3mf");
        assert_eq!(state["batch_selection"]["paths"][0], "kept-source.3mf");
        assert_eq!(state["target"], "qidi");
        assert!(state["result"].is_null());
        assert!(state["output"].is_null());
        assert!(state["batch"].is_null());
        assert_eq!(state["phase"], "idle");
        assert_eq!(state["installed_oem"], true);
        let reopen = registry.execute("slicer.open_selected", &json!({}));
        assert_eq!(reopen["ok"], false);
        assert_eq!(reopen["error"], "convert source before opening slicer");
    }

    #[test]
    fn clear_results_refuses_to_interrupt_running_work() {
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.phase = "running".into();
            state.last_result = Some(json!({"progress":20}));
        }
        assert_eq!(registry.execute("results.clear", &json!({}))["ok"], false);
        assert_eq!(registry.snapshot()["result"]["progress"], 20);
    }
}
