//! Sequential, non-overwriting, cancellable native Rust 3MF batch conversion.
//! It orchestrates the existing verified Registry/3MF worker; no second converter.
use crate::{arg, busy, engine_path, oem, printer_settings, Registry, Session};
use serde_json::{json, Value};
use slicebridge_3mf_probe::{inspect_source_nozzle, validate_native_autonomous_input};
use std::collections::{BTreeMap, BTreeSet};
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
    nozzle: String,
}

#[derive(Clone, Debug)]
struct PreparedItem {
    item: Item,
    source_nozzle: String,
    source_nozzle_assumed: bool,
    // Per-source OEM refusal is retained as a file result; other sources
    // can still use a valid variant, as in the original Go batch.
    resolved: Result<Arc<Value>, String>,
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
    nozzles: &[String],
) -> Result<Vec<Item>, String> {
    if paths.is_empty() || paths.len() > MAX_BATCH {
        return Err(format!(
            "batch requires 1..={MAX_BATCH} distinct 3MF sources"
        ));
    }
    if !output_dir.is_dir() {
        return Err("batch output_dir must be an existing directory".into());
    }
    if paths.len() != nozzles.len() {
        return Err("batch source and physical nozzle counts differ".into());
    }
    let mut sources = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    let mut result = Vec::with_capacity(paths.len());
    for (path, nozzle) in paths.iter().zip(nozzles) {
        if !["0.2", "0.25", "0.4", "0.6", "0.8"].contains(&nozzle.as_str()) {
            return Err(format!(
                "invalid physical nozzle {nozzle:?} for batch source"
            ));
        }
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
            nozzle: nozzle.clone(),
        });
    }
    Ok(result)
}

/// Resolve the same target independently for each 3MF source, mirroring
/// native/batch.go configuredConvertContext(source, target, summary.Nozzle, pref).
/// This is read-only. No worker is started and no output is opened here.
fn prepare_batch<F>(
    paths: &[PathBuf],
    output_dir: &Path,
    target: &str,
    settings: &Value,
    mut resolver: F,
) -> Result<Vec<PreparedItem>, String>
where
    F: FnMut(&str, &str, &BTreeMap<String, String>) -> Result<Value, String>,
{
    if paths.is_empty() || paths.len() > MAX_BATCH {
        return Err(format!("batch requires 1..={MAX_BATCH} source files"));
    }
    if !output_dir.is_dir() {
        return Err("batch output_dir must be an existing directory".into());
    }
    let mut metadata = Vec::with_capacity(paths.len());
    let mut variant_nozzles = Vec::with_capacity(paths.len());
    // Every source uses the same saved printer preferences, but a different
    // source nozzle may select a different *installed* OEM variant.
    let mut verified_bundles: BTreeMap<String, Result<Arc<Value>, String>> = BTreeMap::new();
    for path in paths {
        let source =
            inspect_source_nozzle(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let source_nozzle = source["source_nozzle"]
            .as_str()
            .ok_or("source nozzle inspection returned no diameter")?
            .to_owned();
        let assumed = source["source_nozzle_assumed"] == true;
        let (target_nozzle, overrides) =
            printer_settings::profile_choice_for_source(settings, target, "auto", &source_nozzle)
                .map_err(|err| format!("{}: {err}", path.display()))?;
        let resolved = if let Some(cached) = verified_bundles.get(&target_nozzle) {
            cached.clone()
        } else {
            let result = resolver(target, &target_nozzle, &overrides).and_then(|fresh| {
                if fresh["target"] != target || fresh["nozzle"] != target_nozzle {
                    Err(format!(
                        "installed OEM does not match target {target} {target_nozzle}mm"
                    ))
                } else {
                    Ok(Arc::new(fresh))
                }
            });
            verified_bundles.insert(target_nozzle.clone(), result.clone());
            result
        };
        variant_nozzles.push(target_nozzle);
        metadata.push((source_nozzle, assumed, resolved));
    }
    let filenames = plan_batch(paths, output_dir, target, &variant_nozzles)?;
    Ok(filenames
        .into_iter()
        .zip(metadata)
        .map(
            |(item, (source_nozzle, source_nozzle_assumed, resolved))| PreparedItem {
                item,
                source_nozzle,
                source_nozzle_assumed,
                resolved,
            },
        )
        .collect())
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
            if phase == "completed" || phase == "failed" {
                // Go's BatchReply.FilesDone advances even if a single source
                // fails. Keep completed as files processed for existing UI,
                // while tracking successful/failed outputs separately.
                let processed = batch["completed"].as_u64().unwrap_or(0) + 1;
                batch["completed"] = json!(processed);
                let key = if phase == "completed" {
                    "successes"
                } else {
                    "failures"
                };
                let count = batch[key].as_u64().unwrap_or(0) + 1;
                batch[key] = json!(count);
                let total = batch["total"].as_u64().unwrap_or(0);
                batch["progress_percent"] =
                    json!((processed * 100).checked_div(total).unwrap_or(0));
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

fn verify_source_nozzle_unchanged(ready: &PreparedItem) -> Result<(), String> {
    let latest = inspect_source_nozzle(&ready.item.source)?;
    if latest["source_nozzle"] != ready.source_nozzle
        || latest["source_nozzle_assumed"] != ready.source_nozzle_assumed
    {
        return Err("source nozzle changed since batch OEM resolution".into());
    }
    Ok(())
}

fn run_batch(
    shared: Arc<Mutex<Session>>,
    items: Vec<PreparedItem>,
    engine: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    for (index, ready) in items.iter().enumerate() {
        let item = &ready.item;
        if cancel.load(Ordering::SeqCst) {
            finish(&shared, "cancelled", Some("batch cancelled between items"));
            return;
        }
        update(&shared, index, "running", None);
        let result: Result<Value, String> = (|| {
            // A staged source may have changed after its read-only planning.
            // Never apply a stale OEM variant to a different source nozzle.
            // A missing installed profile is a per-source failure, not a
            // reason to drop the rest of the original batch.
            let bundle = ready.resolved.as_ref().map_err(|error| error.clone())?;
            verify_source_nozzle_unchanged(ready)?;
            // Only trusted installed OEM inheritance. Revalidate the
            // resolved profile against EACH source's own slot materials.
            let validation = validate_native_autonomous_input(bundle.as_ref(), &item.source)?;
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
                session.bundle = Some(bundle.as_ref().clone());
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
                            "source_nozzle":ready.source_nozzle,
                            "source_nozzle_assumed":ready.source_nozzle_assumed,
                            "target_nozzle":item.nozzle,
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
                // A per-file profile/material/source error is not a global
                // batch failure in original Go: report it and keep processing.
                // Cancellation remains terminal and marks queued work skipped.
                if cancel.load(Ordering::SeqCst) {
                    update(&shared, index, "cancelled", Some(json!({"error":error})));
                    finish(&shared, "cancelled", Some(&error));
                    return;
                }
                update(&shared, index, "failed", Some(json!({"error":error})));
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
        // No writes or converter process here. The original Go batch resolves
        // each file's source nozzle independently, not just the first source.
        let settings = printer_settings::current()?;
        let items = prepare_batch(&sources, &output_dir, &target, &settings, oem::resolve)?;
        let batch_nozzle = if items
            .iter()
            .all(|ready| ready.item.nozzle == items[0].item.nozzle)
        {
            items[0].item.nozzle.clone()
        } else {
            "mixed".to_owned()
        };
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
            if printer_settings::current()? != settings {
                return Err("printer settings changed during batch profile resolution".into());
            }
            if params.get("paths").is_none() && session.batch_staged != sources {
                return Err("staged batch sources changed during profile resolution".into());
            }
            if params.get("output_dir").is_none()
                && session.batch_output_dir.as_ref() != Some(&output_dir)
            {
                return Err("batch output directory changed during profile resolution".into());
            }
            session.batch = Some(json!({
                "phase":"running",
                "target":target,"nozzle":batch_nozzle,
                "total":items.len(),"completed":0,"successes":0,"failures":0,
                "progress_percent":0,
                "active_index":null,"error":null,
                "items":items.iter().map(|ready|json!({
                    "source":ready.item.source,"output":ready.item.output,
                    "nozzle":ready.item.nozzle,
                    "source_nozzle":ready.source_nozzle,
                    "source_nozzle_assumed":ready.source_nozzle_assumed,
                    "phase":"queued","receipt":null
                })).collect::<Vec<_>>()
            }));
            session.batch_cancel = Some(cancel.clone());
        }
        let shared = self.state.clone();
        let count = items.len();
        thread::spawn(move || run_batch(shared, items, engine, cancel));
        Ok(json!({"started":true,"phase":"running","count":count,
            "target":target,"nozzle":batch_nozzle,"output_dir":output_dir}))
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
    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }
    fn output_folder(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("sb-batch-nozzles-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn mixed_source_nozzles_receive_distinct_oem_variants_without_conversion() {
        let root = output_folder("mixed");
        let paths = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.6.3mf"),
            fixture("bambu-one-color.3mf"),
            fixture("bambu-nozzle-0.8.3mf"),
        ];
        let before = paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let mut resolved = Vec::<String>::new();
        let ready = prepare_batch(
            &paths,
            &root,
            "u1",
            &printer_settings::defaults(),
            |target, nozzle, overrides| {
                assert!(overrides.is_empty());
                resolved.push(nozzle.to_owned());
                Ok(json!({"target":target,"nozzle":nozzle,"profile":{"test":"only"}}))
            },
        )
        .unwrap();
        assert_eq!(resolved, vec!["0.2", "0.6", "0.4", "0.8"]);
        assert_eq!(
            ready
                .iter()
                .map(|v| v.item.nozzle.as_str())
                .collect::<Vec<_>>(),
            vec!["0.2", "0.6", "0.4", "0.8"]
        );
        assert_eq!(
            ready
                .iter()
                .map(|v| v.source_nozzle.as_str())
                .collect::<Vec<_>>(),
            vec!["0.2", "0.6", "0.4", "0.8"]
        );
        assert!(ready[2].source_nozzle_assumed);
        assert!(!ready[0].source_nozzle_assumed);
        assert!(ready[1]
            .item
            .output
            .to_string_lossy()
            .ends_with("-u1-0_6.3mf"));
        assert!(ready.iter().all(|v| !v.item.output.exists()));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixed_physical_nozzle_wins_and_reuses_single_installed_variant() {
        let root = output_folder("fixed");
        let paths = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.8.3mf"),
        ];
        let mut settings = printer_settings::defaults();
        settings["printers"]["u1"]["nozzle"] = json!("0.6");
        let mut calls = 0;
        let ready = prepare_batch(&paths, &root, "u1", &settings, |target, nozzle, _| {
            assert_eq!(target, "u1");
            assert_eq!(nozzle, "0.6");
            calls += 1;
            Ok(json!({"target":target,"nozzle":nozzle}))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(Arc::ptr_eq(
            ready[0].resolved.as_ref().unwrap(),
            ready[1].resolved.as_ref().unwrap()
        ));
        assert!(ready
            .iter()
            .all(|v| v.item.nozzle == "0.6" && !v.item.output.exists()));
        assert_eq!(ready[0].source_nozzle, "0.2");
        assert_eq!(ready[1].source_nozzle, "0.8");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_nozzle_reuses_one_profile_and_original_narrow_remap_is_preserved() {
        let root = output_folder("reuse");
        let paths = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.2-overheight.3mf"),
        ];
        let mut calls = 0;
        let ready = prepare_batch(
            &paths,
            &root,
            "c5",
            &printer_settings::defaults(),
            |target, nozzle, _| {
                calls += 1;
                assert_eq!((target, nozzle), ("c5", "0.25"));
                Ok(json!({"target":target,"nozzle":nozzle}))
            },
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert!(Arc::ptr_eq(
            ready[0].resolved.as_ref().unwrap(),
            ready[1].resolved.as_ref().unwrap()
        ));
        assert!(ready.iter().all(|v| v.item.nozzle == "0.25"));
        assert!(ready.iter().all(|v| !v.item.output.exists()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_batch_is_rejected_before_inspection_or_oem_resolution() {
        let root = output_folder("oversized");
        let missing = vec![PathBuf::from("does-not-exist.3mf"); 201];
        let error = prepare_batch(
            &missing,
            &root,
            "u1",
            &printer_settings::defaults(),
            |_, _, _| panic!("oversized batch must not inspect/resolve files"),
        )
        .unwrap_err();
        assert!(error.contains("1..=200"), "{error}");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_nozzle_or_assumption_drift_is_refused_before_worker_start() {
        let root = output_folder("drift");
        let paths = [fixture("bambu-nozzle-0.2.3mf")];
        let mut ready = prepare_batch(
            &paths,
            &root,
            "u1",
            &printer_settings::defaults(),
            |target, nozzle, _| Ok(json!({"target":target,"nozzle":nozzle})),
        )
        .unwrap();
        assert!(verify_source_nozzle_unchanged(&ready[0]).is_ok());
        ready[0].source_nozzle = "0.6".into();
        assert!(verify_source_nozzle_unchanged(&ready[0])
            .unwrap_err()
            .contains("source nozzle changed"));
        ready[0].source_nozzle = "0.2".into();
        ready[0].source_nozzle_assumed = true;
        assert!(verify_source_nozzle_unchanged(&ready[0]).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_or_mismatched_oem_is_reported_per_file_without_conversion() {
        let root = output_folder("missing");
        let paths = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.6.3mf"),
        ];
        let settings = printer_settings::defaults();
        let failed = prepare_batch(&paths, &root, "u1", &settings, |target, nozzle, _| {
            if nozzle == "0.6" {
                Err("installed OEM 0.6 not found".into())
            } else {
                Ok(json!({"target":target,"nozzle":nozzle}))
            }
        })
        .unwrap();
        assert!(failed[0].resolved.is_ok());
        assert!(failed[1]
            .resolved
            .as_ref()
            .unwrap_err()
            .contains("0.6 not found"));
        assert!(failed.iter().all(|item| !item.item.output.exists()));
        let mismatch = prepare_batch(&paths[..1], &root, "u1", &settings, |target, _, _| {
            Ok(json!({"target":target,"nozzle":"0.4"}))
        })
        .unwrap();
        assert!(mismatch[0]
            .resolved
            .as_ref()
            .unwrap_err()
            .contains("does not match"));
        let invalid = root.join("not-a-3mf.3mf");
        fs::write(&invalid, b"invalid source").unwrap();
        assert!(prepare_batch(
            &[invalid.clone()],
            &root,
            "u1",
            &settings,
            |_, _, _| panic!("an invalid source must never resolve an OEM")
        )
        .is_err());
        assert!(root.join("not-a-3mf-slicebridge-u1-0_4.3mf").exists() == false);
        assert_eq!(fs::read(&invalid).unwrap(), b"invalid source");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_oem_files_finish_as_individual_failures_without_starting_engine() {
        let root = output_folder("all-unresolved");
        let sources = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.6.3mf"),
        ];
        let ready = prepare_batch(
            &sources,
            &root,
            "u1",
            &printer_settings::defaults(),
            |_, nozzle, _| Err(format!("OEM {nozzle} unavailable")),
        )
        .unwrap();
        assert!(ready.iter().all(|entry| entry.resolved.is_err()));
        let shared = Arc::new(Mutex::new(Session::default()));
        {
            let mut state = shared.lock().unwrap();
            state.batch = Some(json!({
                "phase":"running","total":2,"completed":0,"successes":0,"failures":0,
                "progress_percent":0,"items":[
                    {"phase":"queued","receipt":null},
                    {"phase":"queued","receipt":null}
                ]
            }));
        }
        // This fake engine path intentionally does not exist. Every unresolved
        // profile is reported before any worker process can be invoked.
        run_batch(
            shared.clone(),
            ready,
            PathBuf::from("MUST-NOT-EXECUTE"),
            Arc::new(AtomicBool::new(false)),
        );
        let state = shared.lock().unwrap();
        let batch = state.batch.as_ref().unwrap();
        assert_eq!(batch["phase"], "completed");
        assert_eq!(batch["completed"], 2);
        assert_eq!(batch["successes"], 0);
        assert_eq!(batch["failures"], 2);
        assert_eq!(batch["progress_percent"], 100);
        assert!(batch["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["phase"] == "failed"
                && entry["receipt"]["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("unavailable"))));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filenames_are_deterministic_and_never_overwritten() {
        let base = std::env::temp_dir().join(format!("sb-batch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let source = base.join("design.3mf");
        fs::write(&source, b"not-a-real-3mf").unwrap();
        let items = plan_batch(
            std::slice::from_ref(&source),
            &base,
            "qidi",
            &["0.6".into()],
        )
        .unwrap();
        let output = base.join("design-slicebridge-qidi-0_6.3mf");
        assert_eq!(items[0].output, output);
        assert!(plan_batch(
            &[source.clone(), source.clone()],
            &base,
            "qidi",
            &["0.6".into(), "0.6".into()]
        )
        .is_err());
        fs::write(&output, b"already exists").unwrap();
        assert!(plan_batch(&[source], &base, "qidi", &["0.6".into()]).is_err());
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
        let err = plan_batch(&paths, &dest, "u1", &["0.4".into(), "0.4".into()]).unwrap_err();
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
    fn a_failed_source_keeps_later_files_queued_and_reports_all_outcomes() {
        let shared = Arc::new(Mutex::new(Session::default()));
        {
            let mut state = shared.lock().unwrap();
            state.batch = Some(json!({
                "phase":"running","total":3,
                "completed":0,"successes":0,"failures":0,"progress_percent":0,
                "items":[
                    {"phase":"queued","receipt":null},
                    {"phase":"queued","receipt":null},
                    {"phase":"queued","receipt":null}
                ]
            }));
        }
        update(&shared, 0, "running", None);
        update(
            &shared,
            0,
            "failed",
            Some(json!({"error":"source profile unavailable"})),
        );
        {
            let state = shared.lock().unwrap();
            let batch = state.batch.as_ref().unwrap();
            assert_eq!(batch["phase"], "running");
            assert_eq!(batch["items"][1]["phase"], "queued");
            assert_eq!(batch["completed"], 1);
            assert_eq!(batch["successes"], 0);
            assert_eq!(batch["failures"], 1);
            assert_eq!(batch["progress_percent"], 33);
        }
        update(&shared, 1, "completed", Some(json!({"bytes":200})));
        update(&shared, 2, "completed", Some(json!({"bytes":300})));
        finish(&shared, "completed", None);
        let state = shared.lock().unwrap();
        let batch = state.batch.as_ref().unwrap();
        assert_eq!(batch["phase"], "completed");
        assert_eq!(batch["completed"], 3);
        assert_eq!(batch["successes"], 2);
        assert_eq!(batch["failures"], 1);
        assert_eq!(batch["progress_percent"], 100);
        assert_eq!(batch["items"][0]["phase"], "failed");
        assert_eq!(
            batch["items"][0]["receipt"]["error"],
            "source profile unavailable"
        );
        assert_eq!(batch["items"][1]["phase"], "completed");
        assert_eq!(batch["items"][2]["phase"], "completed");
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
