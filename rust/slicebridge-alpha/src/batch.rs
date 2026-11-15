//! Sequential, non-overwriting, cancellable native Rust 3MF batch conversion.
//! It orchestrates the existing verified Registry/3MF worker; no second converter.
use crate::{arg, busy, engine_path, Registry, Session};
use serde_json::{json, Value};
use slicebridge_3mf_probe::validate_native_autonomous_input;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// Match the stable SliceBridge queue limit, not the experimental 32-file UI.
const MAX_BATCH: usize = 200;

#[derive(Clone, Debug)]
struct Item {
    source: PathBuf,
    output: PathBuf,
}

fn parse_sources(params: &Value) -> Result<Vec<PathBuf>, String> {
    let values = params
        .get("paths")
        .and_then(Value::as_array)
        .ok_or("batch requires paths array")?;
    if values.is_empty() || values.len() > MAX_BATCH {
        return Err(format!("batch requires 1..={MAX_BATCH} source files"));
    }
    values
        .iter()
        .map(|value| {
            let path = value
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or("batch paths must be nonempty strings")?;
            Ok(PathBuf::from(path))
        })
        .collect()
}

fn plan_batch(
    paths: &[PathBuf],
    output_dir: &Path,
    target: &str,
    nozzle: &str,
) -> Result<Vec<Item>, String> {
    if paths.is_empty() || paths.len() > MAX_BATCH {
        return Err(format!(
            "batch requires 1..={MAX_BATCH} distinct 3MF sources"
        ));
    }
    if !output_dir.is_dir() {
        return Err("batch output_dir must be an existing directory".into());
    }
    let mut sources = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    let mut result = Vec::with_capacity(paths.len());
    for path in paths {
        if !path.is_file() {
            return Err(format!(
                "batch source is not a regular file: {}",
                path.display()
            ));
        }
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
        {
            return Err(format!("batch source must end in .3mf: {}", path.display()));
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("batch source canonicalization: {e}"))?;
        if !sources.insert(canonical) {
            return Err(format!("duplicate 3MF source: {}", path.display()));
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or("batch source filename is not UTF-8")?;
        let suffix = nozzle.replace('.', "_");
        let destination = output_dir.join(format!("{stem}-slicebridge-{target}-{suffix}.3mf"));
        // Windows output comparison is case-insensitive, as is filesystem naming.
        let key = destination.to_string_lossy().to_lowercase();
        if !destinations.insert(key) {
            return Err(format!(
                "batch output filename collision: {}",
                destination.display()
            ));
        }
        if destination.exists() {
            return Err(format!(
                "batch refuses existing output: {}",
                destination.display()
            ));
        }
        if crate::same_path(path, &destination) {
            return Err("batch output cannot be the original source".into());
        }
        result.push(Item {
            source: path.clone(),
            output: destination,
        });
    }
    Ok(result)
}

fn update(shared: &Arc<Mutex<Session>>, index: usize, phase: &str, detail: Option<Value>) {
    if let Ok(mut session) = shared.lock() {
        if let Some(batch) = session.batch.as_mut() {
            batch["active_index"] = if phase == "running" {
                json!(index)
            } else {
                Value::Null
            };
            batch["items"][index]["phase"] = json!(phase);
            if let Some(detail) = detail {
                batch["items"][index]["receipt"] = detail;
            }
            if phase == "completed" {
                let completed = batch["completed"].as_u64().unwrap_or(0);
                let done = completed + 1;
                batch["completed"] = json!(done);
                let total = batch["total"].as_u64().unwrap_or(0);
                batch["progress_percent"] = json!((done * 100).checked_div(total).unwrap_or(0));
            }
        }
    }
}

fn finish(shared: &Arc<Mutex<Session>>, phase: &str, error: Option<&str>) {
    if let Ok(mut session) = shared.lock() {
        if let Some(batch) = session.batch.as_mut() {
            batch["phase"] = json!(phase);
            batch["active_index"] = Value::Null;
            if phase == "cancelled" || phase == "failed" {
                if let Some(items) = batch["items"].as_array_mut() {
                    for item in items {
                        if item["phase"] == "queued" {
                            item["phase"] = json!("skipped");
                        }
                    }
                }
            }
            if let Some(error) = error {
                batch["error"] = json!(error);
            }
        }
        session.batch_cancel = None;
    }
}

fn run_batch(
    shared: Arc<Mutex<Session>>,
    items: Vec<Item>,
    bundle: Value,
    engine: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    for (index, item) in items.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            finish(&shared, "cancelled", Some("batch cancelled between items"));
            return;
        }
        update(&shared, index, "running", None);
        let result: Result<Value, String> = (|| {
            // Only trusted installed OEM inheritance. Revalidate the
            // resolved profile against EACH source's own slot materials.
            let validation = validate_native_autonomous_input(&bundle, &item.source)?;
            if cancel.load(Ordering::SeqCst) {
                return Err("batch cancelled during preflight".into());
            }
            let worker = Registry::new();
            let selected = worker.execute("source.select", &json!({"path":item.source}));
            if selected["ok"] != true {
                return Err(format!("source inspection failed: {}", selected["error"]));
            }
            {
                let mut session = worker.state.lock().map_err(|e| e.to_string())?;
                session.bundle = Some(bundle.clone());
            }
            let start = worker.execute(
                "conversion.start",
                &json!({"output":item.output,"engine":engine}),
            );
            if start["ok"] != true {
                return Err(format!("worker failed to start: {}", start["error"]));
            }
            loop {
                if cancel.load(Ordering::SeqCst) {
                    worker.execute("conversion.cancel", &json!({}));
                }
                let status = worker.snapshot();
                match status["phase"].as_str().unwrap_or("unknown") {
                    "running" => thread::sleep(Duration::from_millis(80)),
                    "completed" => {
                        // If cancellation arrives just after the native worker has
                        // atomically completed, keep that finished output in the
                        // receipt. The next iteration observes cancel and stops.
                        let converted = status["output"]
                            .as_str()
                            .ok_or("conversion completed without output file")?;
                        if converted != item.output.to_string_lossy() {
                            return Err("batch worker output path differs from planned path".into());
                        }
                        if !item.output.is_file() {
                            return Err("batch worker output file missing".into());
                        }
                        return Ok(json!({
                            "validation": validation,
                            "conversion":status["result"],
                            "bytes":fs::metadata(&item.output).map_err(|e|e.to_string())?.len()
                        }));
                    }
                    "failed" | "cancelled" => {
                        return Err(format!(
                            "conversion {}: {}",
                            status["phase"], status["result"]
                        ));
                    }
                    phase => return Err(format!("unexpected conversion phase: {phase}")),
                }
            }
        })();
        match result {
            Ok(receipt) => update(&shared, index, "completed", Some(receipt)),
            Err(error) => {
                update(
                    &shared,
                    index,
                    if cancel.load(Ordering::SeqCst) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    Some(json!({"error":error})),
                );
                finish(
                    &shared,
                    if cancel.load(Ordering::SeqCst) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    Some(&error),
                );
                return;
            }
        }
    }
    finish(&shared, "completed", None);
}

impl Registry {
    pub(crate) fn batch_clear(&self) -> Result<Value, String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        if busy(&state) {
            return Err("cannot clear sources during conversion".into());
        }
        let removed = state
            .batch_staged
            .len()
            .max(usize::from(state.source.is_some()));
        state.batch_staged.clear();
        state.source = None;
        state.facts = None;
        state.bundle_path = None;
        state.bundle_label = None;
        state.bundle = None;
        state.installed_oem = false;
        state.target = None;
        state.nozzle = None;
        state.selected_materials.clear();
        state.output = None;
        state.last_result = None;
        state.batch = None;
        state.phase = "idle".into();
        Ok(json!({"removed": removed}))
    }

    pub(crate) fn batch_stage(&self, params: &Value) -> Result<Value, String> {
        let paths = parse_sources(params)?;
        let mut canonical = BTreeSet::new();
        for path in &paths {
            if !path.is_file()
                || !path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
            {
                return Err(format!(
                    "batch stage expects existing .3mf: {}",
                    path.display()
                ));
            }
            if !canonical.insert(path.canonicalize().map_err(|e| e.to_string())?) {
                return Err(format!("duplicate batch source: {}", path.display()));
            }
        }
        let mut session = self.state.lock().map_err(|e| e.to_string())?;
        if busy(&session) {
            return Err("cannot stage new sources while a conversion is active".into());
        }
        session.batch_staged = paths;
        Ok(json!({"staged":session.batch_staged.len(),
            "paths":session.batch_staged}))
    }

    pub(crate) fn batch_directory(&self, params: &Value) -> Result<Value, String> {
        let path = PathBuf::from(arg(params, "path")?);
        if !path.is_dir() {
            return Err("batch directory must exist".into());
        }
        let mut session = self.state.lock().map_err(|e| e.to_string())?;
        if busy(&session) {
            return Err("cannot change batch directory while a conversion is active".into());
        }
        session.batch_output_dir = Some(path.clone());
        Ok(json!({"output_dir":path}))
    }

    pub(crate) fn batch_selection(&self) -> Result<Value, String> {
        let session = self.state.lock().map_err(|e| e.to_string())?;
        Ok(json!({"paths":session.batch_staged,
            "output_dir":session.batch_output_dir}))
    }

    pub(crate) fn batch_start(&self, params: &Value) -> Result<Value, String> {
        // An explicit batch.start payload remains backwards-compatible with CLI/MCP.
        // With no payload the same command starts the agent-visible staged selection.
        let (sources, output_dir) = {
            let session = self.state.lock().map_err(|e| e.to_string())?;
            let sources = if params.get("paths").is_some() {
                parse_sources(params)?
            } else {
                session.batch_staged.clone()
            };
            let output_dir = if params.get("output_dir").is_some() {
                PathBuf::from(arg(params, "output_dir")?)
            } else {
                session
                    .batch_output_dir
                    .clone()
                    .ok_or("batch output directory not selected")?
            };
            (sources, output_dir)
        };
        let engine = engine_path(params.get("engine").and_then(Value::as_str))?;
        if !engine.is_file() {
            return Err(format!("batch Rust engine missing: {}", engine.display()));
        }
        let (bundle, target, nozzle) = {
            let session = self.state.lock().map_err(|e| e.to_string())?;
            if busy(&session) {
                return Err("cannot start batch while conversion or batch is active".into());
            }
            if !session.installed_oem {
                return Err("batch requires selected installed OEM printer".into());
            }
            (
                session.bundle.clone().ok_or("batch OEM bundle missing")?,
                session.target.clone().ok_or("batch OEM target missing")?,
                session.nozzle.clone().ok_or("batch OEM nozzle missing")?,
            )
        };
        let items = plan_batch(&sources, &output_dir, &target, &nozzle)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut session = self.state.lock().map_err(|e| e.to_string())?;
            if busy(&session)
                || !session.installed_oem
                || session.target.as_deref() != Some(&target)
                || session.nozzle.as_deref() != Some(&nozzle)
                || session.bundle.as_ref() != Some(&bundle)
            {
                return Err("OEM selection or session changed during batch preparation".into());
            }
            session.batch = Some(json!({
                "phase":"running",
                "target":target,"nozzle":nozzle,
                "total":items.len(),"completed":0,"progress_percent":0,
                "active_index":null,"error":null,
                "items":items.iter().map(|item|json!({
                    "source":item.source,"output":item.output,
                    "phase":"queued","receipt":null
                })).collect::<Vec<_>>()
            }));
            session.batch_cancel = Some(cancel.clone());
        }
        let shared = self.state.clone();
        let count = items.len();
        thread::spawn(move || run_batch(shared, items, bundle, engine, cancel));
        Ok(json!({"started":true,"phase":"running","count":count,
            "target":target,"nozzle":nozzle,"output_dir":output_dir}))
    }

    pub(crate) fn batch_cancel(&self) -> Result<Value, String> {
        let session = self.state.lock().map_err(|e| e.to_string())?;
        let phase = session
            .batch
            .as_ref()
            .and_then(|v| v["phase"].as_str())
            .unwrap_or("idle");
        if phase == "running" {
            if let Some(flag) = &session.batch_cancel {
                flag.store(true, Ordering::SeqCst);
            }
            Ok(json!({"cancel_requested":true,"phase":"cancelling"}))
        } else {
            Ok(json!({"cancel_requested":false,"phase":phase}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_oem_fails_before_batch_creates_output() {
        let registry = Registry::new();
        let result = registry.execute(
            "batch.start",
            &json!({"paths":["foo.3mf"],"output_dir":"."}),
        );
        assert_eq!(result["ok"], false);
        assert!(registry.snapshot()["batch"].is_null());
    }
    #[test]
    fn filenames_are_deterministic_and_never_overwritten() {
        let base = std::env::temp_dir().join(format!("sb-batch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let source = base.join("design.3mf");
        fs::write(&source, b"not-a-real-3mf").unwrap();
        let items = plan_batch(std::slice::from_ref(&source), &base, "qidi", "0.6").unwrap();
        let output = base.join("design-slicebridge-qidi-0_6.3mf");
        assert_eq!(items[0].output, output);
        assert!(plan_batch(&[source.clone(), source.clone()], &base, "qidi", "0.6").is_err());
        fs::write(&output, b"already exists").unwrap();
        assert!(plan_batch(&[source], &base, "qidi", "0.6").is_err());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn same_stem_in_different_source_folders_is_rejected() {
        let root = std::env::temp_dir().join(format!("sb-batch-stem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sources = [root.join("one"), root.join("two")];
        let dest = root.join("out");
        for folder in &sources {
            fs::create_dir_all(folder).unwrap();
            fs::write(folder.join("same.3mf"), b"fixture").unwrap();
        }
        fs::create_dir_all(&dest).unwrap();
        let paths = sources.map(|dir| dir.join("same.3mf"));
        let err = plan_batch(&paths, &dest, "u1", "0.4").unwrap_err();
        assert!(err.contains("collision"), "{err}");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn staging_and_directory_share_agent_registry_without_starting_conversion() {
        let root = std::env::temp_dir().join(format!("sb-batch-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let one = root.join("one.3mf");
        fs::write(&one, b"not-a-real-3mf").unwrap();
        let registry = Registry::new();
        assert_eq!(
            registry.execute("batch.stage", &json!({"paths":[one]}))["ok"],
            true
        );
        assert_eq!(
            registry.execute("batch.directory", &json!({"path":root}))["ok"],
            true
        );
        assert_eq!(
            registry.execute("batch.selection", &json!({}))["data"]["paths"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            registry.snapshot()["batch_selection"]["paths"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            registry.execute("batch.stage", &json!({"paths":[one,one]}))["ok"],
            false
        );
        assert_eq!(registry.execute("batch.start", &json!({}))["ok"], false);
        assert!(registry.snapshot()["batch"].is_null());
        assert!(one.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_batch_marks_unprocessed_items_as_skipped() {
        let shared = Arc::new(Mutex::new(Session::default()));
        {
            let mut s = shared.lock().unwrap();
            s.batch = Some(json!({"phase":"running","active_index":0,
                "items":[{"phase":"failed"},{"phase":"queued"},{"phase":"queued"}]}));
        }
        finish(&shared, "failed", Some("fixture failure"));
        let s = shared.lock().unwrap();
        let batch = s.batch.as_ref().unwrap();
        assert_eq!(batch["items"][1]["phase"], "skipped");
        assert_eq!(batch["items"][2]["phase"], "skipped");
        assert_eq!(batch["error"], "fixture failure");
    }

    #[test]
    fn malformed_and_empty_batch_requests_are_rejected() {
        let r = Registry::new();
        for v in [
            json!({}),
            json!({"paths":[],"output_dir":"."}),
            json!({"paths":[null],"output_dir":"."}),
            json!({"paths":(0..201).map(|_|"x.3mf").collect::<Vec<_>>(),"output_dir":"."}),
        ] {
            assert_eq!(r.execute("batch.start", &v)["ok"], false);
        }
        assert_eq!(
            r.execute("batch.cancel", &json!({}))["data"]["cancel_requested"],
            false
        );
    }
}

#[cfg(test)]
mod original_queue_parity_tests {
    use super::*;

    #[test]
    fn source_queue_has_the_stable_main_200_file_limit() {
        let paths: Vec<String> = (0..200)
            .map(|index| format!("fixture-{index}.3mf"))
            .collect();
        assert_eq!(parse_sources(&json!({"paths": paths})).unwrap().len(), 200);
        let overflow: Vec<String> = (0..201)
            .map(|index| format!("fixture-{index}.3mf"))
            .collect();
        assert!(parse_sources(&json!({"paths":overflow}))
            .unwrap_err()
            .contains("1..=200"));
    }

    #[test]
    fn clearing_source_queue_preserves_files_and_does_not_start_conversion() {
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.source = Some(PathBuf::from("source.3mf"));
            state.batch_staged = vec![PathBuf::from("source.3mf")];
            state.facts = Some(json!({"triangles": 8}));
            state.phase = "idle".into();
        }
        let reply = registry.execute("batch.clear", &json!({}));
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["data"]["removed"], 1);
        assert!(registry.snapshot()["source"].is_null());
        assert!(registry.snapshot()["facts"].is_null());
        assert!(registry.snapshot()["batch_selection"]["paths"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(registry.snapshot()["batch"].is_null());
    }

    #[test]
    fn clearing_is_rejected_while_conversion_is_running() {
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.phase = "running".into();
            state.batch_staged = vec![PathBuf::from("source.3mf")];
        }
        assert_eq!(registry.execute("batch.clear", &json!({}))["ok"], false);
        assert_eq!(
            registry.snapshot()["batch_selection"]["paths"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
