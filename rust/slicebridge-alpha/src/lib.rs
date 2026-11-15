//! One Rust command surface for the desktop and agent transports.
//! The proven 3MF implementation remains in slicebridge-3mf-probe.
pub mod batch;
pub mod my_printers;
pub mod oem;
pub mod printer_settings;
pub mod protocol;
use serde_json::{json, Value};
use slicebridge_3mf_probe::{inspect_package, validate_native_autonomous_input};
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

#[derive(Default)]
struct Session {
    source: Option<PathBuf>,
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
        "settings.get",
        "Read validated stable printer settings or defaults without modifying installed settings",
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
            "settings.get" => printer_settings::current(),
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
            "results.clear" => {
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if busy(&state) {
                    return Err("cannot clear results during conversion".into());
                }
                state.last_result = None;
                state.batch = None;
                if state.phase != "idle" {
                    state.phase = "idle".into();
                }
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
                let receipt = json!({
                    "path": path.to_string_lossy(),
                    "vertices": facts.vertices,
                    "triangles": facts.triangles,
                    "slice_caches": facts.slice_caches,
                    "printer_model": facts.printer_model,
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
                let nozzle = params
                    .get("nozzle")
                    .and_then(Value::as_str)
                    .unwrap_or("0.4")
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
                let bundle = oem::resolve(&target, &nozzle, &BTreeMap::new())?;
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
                let (target, nozzle, source, mut overrides) = {
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
                    )
                };
                overrides.insert(material.clone(), preset.clone());
                // Rebuild the entire bundle from the same installed OEM inheritance;
                // no display-only state or copied vendor material constants.
                let bundle = oem::resolve(&target, &nozzle, &overrides)?;
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
                if !slicer.is_file() {
                    return Err("slicer executable does not exist".into());
                }
                launch_slicer(&slicer, &output)?;
                Ok(json!({"launched": true, "slicer": slicer.to_string_lossy(),
                    "file": output.to_string_lossy(), "slicing": false}))
            }
            "slicer.open_selected" => {
                let state = self.state.lock().map_err(|e| e.to_string())?;
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
                let slicer = oem::installed_executable(target, nozzle)?;
                drop(state);
                launch_slicer(&slicer, &output)?;
                Ok(json!({"launched":true,"slicer":slicer.to_string_lossy(),
                    "file":output.to_string_lossy(),"slicing":false}))
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

fn launch_slicer(executable: &Path, output: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut launch = {
        let mut cmd = Command::new("open");
        cmd.arg("-a").arg(executable).arg("--args").arg(output);
        cmd
    };
    #[cfg(not(target_os = "macos"))]
    let mut launch = {
        let mut cmd = Command::new(executable);
        cmd.arg(output);
        cmd
    };
    launch
        .spawn()
        .map_err(|e| format!("could not open selected slicer: {e}"))?;
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
    fn nonexistent_source_does_not_change_state() {
        let registry = Registry::new();
        let result = registry.execute("source.select", &json!({"path": "nonexistent.3mf"}));
        assert_eq!(result["ok"], false);
        assert_eq!(registry.snapshot()["source"], Value::Null);
    }

    #[test]
    fn conversion_requires_verified_bundle() {
        let registry = Registry::new();
        let result = registry.execute("conversion.start", &json!({"output": "out.3mf"}));
        assert_eq!(result["ok"], false);
        assert_eq!(registry.snapshot()["phase"], "idle");
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
    fn opening_selected_slicer_requires_completed_conversion() {
        let registry = Registry::new();
        let result = registry.execute("slicer.open_selected", &json!({}));
        assert_eq!(result["ok"], false);
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
            state.batch = Some(json!({"phase":"completed","total":1}));
        }
        let result = registry.execute("results.clear", &json!({}));
        assert_eq!(result["ok"], true);
        let state = registry.snapshot();
        assert_eq!(state["source"], "kept-source.3mf");
        assert_eq!(state["batch_selection"]["paths"][0], "kept-source.3mf");
        assert_eq!(state["target"], "qidi");
        assert!(state["result"].is_null());
        assert!(state["batch"].is_null());
        assert_eq!(state["phase"], "idle");
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
