//! One Rust command surface for the desktop and agent transports.
//! The proven 3MF implementation remains in slicebridge-3mf-probe.
pub mod protocol;
use serde_json::{json, Value};
use slicebridge_3mf_probe::{inspect_package, validate_native_autonomous_input};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Default)]
struct Session {
    source: Option<PathBuf>,
    facts: Option<Value>,
    bundle_path: Option<PathBuf>,
    bundle_label: Option<String>,
    selected_materials: BTreeMap<usize, String>,
    output: Option<PathBuf>,
    phase: String,
    last_result: Option<Value>,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Default)]
pub struct Registry {
    state: Arc<Mutex<Session>>,
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("commands.list", "Discover typed Rust application commands"),
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
        "bundle.load",
        "Validate a complete, pre-resolved OEM bundle",
    ),
    (
        "printer.select",
        "Select the printer from the validated OEM bundle",
    ),
    (
        "material.select",
        "Record a slot's verified OEM material selection",
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
            "bundle_path": state.bundle_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "printer": state.bundle_label,
            "materials": state.selected_materials,
            "output": state.output.as_ref().map(|p| p.to_string_lossy().to_string()),
            "phase": if state.phase.is_empty() { "idle" } else { &state.phase },
            "result": state.last_result,
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
                    if state.phase == "running" {
                        return Err("cannot change source during conversion".into());
                    }
                    state.source = Some(path);
                    state.facts = Some(receipt.clone());
                    state.last_result = None;
                    state.output = None;
                }
                Ok(receipt)
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
                if state.phase == "running" {
                    return Err("cannot change OEM bundle during conversion".into());
                }
                if state.source.as_ref() != Some(&source) {
                    return Err("source changed during OEM verification".into());
                }
                state.bundle_path = Some(path);
                state.bundle_label = Some(label.clone());
                state.selected_materials.clear();
                Ok(json!({"printer": label, "validation": verified}))
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
                let slot = params
                    .get("slot")
                    .and_then(Value::as_u64)
                    .ok_or("slot must be a positive integer")?;
                if !(1..=64).contains(&slot) {
                    return Err("slot must be 1..64".into());
                }
                let name = arg(params, "name")?;
                let mut state = self.state.lock().map_err(|e| e.to_string())?;
                if state.bundle_path.is_none() {
                    return Err("load a verified OEM bundle before selecting materials".into());
                }
                // This is a visible selection receipt, not a fabricated replacement OEM leaf.
                // Only a fully verified bundle determines conversion parameters.
                state
                    .selected_materials
                    .insert(slot as usize, name.to_string());
                Ok(
                    json!({"slot": slot, "name": name, "note": "display selection only; conversion uses verified OEM bundle"}),
                )
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
                Command::new(&slicer)
                    .arg(&output)
                    .spawn()
                    .map_err(|e| format!("could not open slicer: {e}"))?;
                Ok(json!({"launched": true, "slicer": slicer.to_string_lossy(),
                    "file": output.to_string_lossy(), "slicing": false}))
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
        if state.phase == "running" {
            return Err("conversion is already running".into());
        }
        let source = state.source.clone().ok_or("select a source 3MF first")?;
        let bundle = state
            .bundle_path
            .clone()
            .ok_or("load a verified OEM bundle first")?;
        if same_path(&source, &output) {
            return Err("output must differ from source".into());
        }
        let child = Command::new(&engine)
            .arg("--convert-native-autonomous")
            .arg(&bundle)
            .arg(&source)
            .arg(&output)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Rust engine start failed: {e}"))?;
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
    fn unknown_command_rejected() {
        let result = Registry::new().execute("printer.print", &json!({}));
        assert_eq!(result["ok"], false);
    }
}
