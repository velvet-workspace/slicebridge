//! Sequential, non-overwriting, cancellable native Rust 3MF batch conversion.
//! It orchestrates the existing verified Registry/3MF worker; no second converter.
use crate::{arg, busy, engine_path, oem, printer_settings, same_path, Registry, Session};
use serde_json::{json, Value};
use slicebridge_3mf_probe::{
    inspect_source_details, inspect_source_nozzle, validate_native_autonomous_input, PackageFacts,
};
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
    source_invalid: bool,
    // Per-source OEM refusal is retained as a file result; other sources
    // can still use a valid variant, as in the original Go batch.
    resolved: Result<Arc<Value>, String>,
}

#[derive(Clone, Debug)]
struct PreparedTarget {
    target: String,
    ready: PreparedItem,
}

#[derive(Clone, Debug)]
struct PreparedSource {
    source: PathBuf,
    // Go's sourceSummary reports object/instance counts per source.
    // Capture them once before the Registry lock; invalid files remain per-file failures.
    source_facts: Option<PackageFacts>,
    source_summary: Option<Value>,
    targets: Vec<PreparedTarget>,
}

// Stable native/batch.go validates 1..32 unique printer target IDs.
// Saved My Printers catalogue membership is separate from conversion support.
fn validate_multi_targets(targets: &[String], saved_ids: &[String]) -> Result<(), String> {
    if targets.is_empty() || targets.len() > 32 {
        return Err("select between 1 and 32 printers".into());
    }
    let mut seen = BTreeSet::new();
    for target in targets {
        let catalogue_id = match target.as_str() {
            "u1" => "snapmaker-u1",
            "c5" => "flashforge-creator-5",
            "cc2" => "elegoo-centauri-carbon-2",
            "qidi" => "qidi-x-plus-5",
            _ => return Err(format!("unported printer target {target:?}")),
        };
        if !seen.insert(target) {
            return Err(format!("invalid or repeated target {target:?}"));
        }
        if !saved_ids.iter().any(|id| id == catalogue_id) {
            return Err(format!("target {target:?} is not selected in My Printers"));
        }
    }
    Ok(())
}

fn parse_targets(params: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(value) = params.get("targets") else {
        return Ok(None);
    };
    let items = value.as_array().ok_or("targets must be an array")?;
    if items.is_empty() || items.len() > 32 {
        return Err("select between 1 and 32 printers".into());
    }
    let mut targets = Vec::with_capacity(items.len());
    for value in items {
        let target = value
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("targets must be nonempty printer IDs")?;
        if targets.iter().any(|existing| existing == target) {
            return Err(format!("invalid or repeated target {target:?}"));
        }
        targets.push(target.to_owned());
    }
    Ok(Some(targets))
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

// Source-locked to origin/main native/app.go outputs and native/batch.go
// writeConversion: output is a sibling of the original source, never in an
// unrelated destination directory or decorated with the physical nozzle.
fn original_prefix(target: &str) -> Result<&'static str, String> {
    match target {
        "u1" => Ok("U1"),
        "c5" => Ok("Creator5"),
        "cc2" => Ok("CentauriCarbon2"),
        "qidi" => Ok("XPlus5"),
        _ => Err(format!(
            "no verified original output prefix for target {target:?}"
        )),
    }
}

fn output_key(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

// The injectable existence probe makes all filename collision tests read-only;
// production uses fs::metadata, matching Go's os.Stat behavior.
fn next_original_output_with<F>(
    source: &Path,
    target: &str,
    reserved: &BTreeSet<String>,
    mut exists: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&Path) -> Result<bool, String>,
{
    for attempt in 1..=100 {
        let candidate = original_output_name(source, target, attempt)?;
        if reserved.contains(&output_key(&candidate)) || exists(&candidate)? {
            continue;
        }
        return Ok(candidate);
    }
    Err("OUTPUT_COLLISIONS: Too many same-name outputs".into())
}

// Exact Go native/batch.go filename template, including the 1..=100 limit.
fn original_output_name(source: &Path, target: &str, attempt: usize) -> Result<PathBuf, String> {
    let prefix = original_prefix(target)?;
    let name = source
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or("source filename is not UTF-8")?;
    let stem = source
        .file_stem()
        .and_then(|part| part.to_str())
        .ok_or("source stem is not UTF-8")?;
    let extension = source
        .extension()
        .and_then(|part| part.to_str())
        .ok_or("source extension is not UTF-8")?;
    let directory = source.parent().ok_or("source has no parent directory")?;
    let filename = if attempt == 1 {
        format!("{prefix}-{name}")
    } else {
        format!("{prefix}-{stem}-{attempt}.{extension}")
    };
    Ok(directory.join(filename))
}

#[derive(Debug, PartialEq, Eq)]
enum OutputAttemptError {
    // A different process created the candidate after the existence check.
    // This is the ONLY retryable conversion failure in original Go.
    Exists,
    Other(String),
}

// Read-only-testable Go writeConversion retry logic. The callback may be a
// mocked collision/receipt in tests; the real conversion call lives solely in
// execute_prepared, not in the tests.
fn write_first_available_with<F, E>(
    source: &Path,
    target: &str,
    mut exists: E,
    mut convert: F,
) -> Result<(PathBuf, Value), String>
where
    E: FnMut(&Path) -> Result<bool, String>,
    F: FnMut(&Path) -> Result<Value, OutputAttemptError>,
{
    for attempt in 1..=100 {
        let candidate = original_output_name(source, target, attempt)?;
        if exists(&candidate)? {
            continue;
        }
        match convert(&candidate) {
            Ok(receipt) => return Ok((candidate, receipt)),
            Err(OutputAttemptError::Exists) => continue,
            Err(OutputAttemptError::Other(error)) => return Err(error),
        }
    }
    Err("OUTPUT_COLLISIONS: Too many same-name outputs".into())
}

// The Rust worker carries string errors in a JSON envelope. Match only the
// known no-overwrite failures AND confirm that the output now exists; unrelated
// worker/permissions/material errors must not become filename retries.
fn is_worker_output_exists(receipt: &Value, candidate_exists: bool) -> bool {
    let Some(error) = receipt["engine"]["error"].as_str() else {
        return false;
    };
    let atomic_publish_collision = error
        .strip_prefix("destination not created (no overwrite): ")
        .is_some_and(|detail| {
            let detail = detail.to_ascii_lowercase();
            detail.contains("file exists")
                || detail.contains("already exists")
                || detail.contains("os error 17")
                || detail.contains("os error 80")
                || detail.contains("os error 183")
        });
    candidate_exists
        && (error == "destination already exists; refusing overwrite"
            || error == "destination already exists; refusing to overwrite"
            || atomic_publish_collision)
}

// Preserve stable Go ConvertResult.Code without inventing error categories.
fn failure_receipt(error: &str, source_invalid: bool) -> Value {
    if source_invalid {
        return json!({"code":"SOURCE_INVALID","error":error});
    }
    for code in ["OUTPUT_COLLISIONS", "OUTPUT_INVALID"] {
        if let Some(message) = error.strip_prefix(&format!("{code}: ")) {
            return json!({"code":code,"error":message});
        }
    }
    json!({"error":error})
}

fn output_exists(path: &Path) -> Result<bool, String> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("OUTPUT_INVALID: {err}")),
    }
}

#[cfg(test)]
fn plan_batch(
    paths: &[PathBuf],
    target: &str,
    nozzles: &[Option<String>],
) -> Result<Vec<Item>, String> {
    plan_batch_with(paths, target, nozzles, output_exists)
}

fn plan_batch_with<E>(
    paths: &[PathBuf],
    target: &str,
    // None denotes a source rejected by 3MF inspection or profile selection.
    // It must remain in the batch report, but must not be assigned an output.
    nozzles: &[Option<String>],
    mut exists: E,
) -> Result<Vec<Item>, String>
where
    E: FnMut(&Path) -> Result<bool, String>,
{
    if paths.is_empty() || paths.len() > MAX_BATCH {
        return Err(format!(
            "batch requires 1..={MAX_BATCH} distinct 3MF sources"
        ));
    }
    if paths.len() != nozzles.len() {
        return Err("batch source and physical nozzle counts differ".into());
    }
    let mut sources = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    let mut result = Vec::with_capacity(paths.len());
    for (path, nozzle) in paths.iter().zip(nozzles) {
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
        let Some(nozzle) = nozzle.as_deref() else {
            // Failed preflight: no destination is reserved or ever written.
            result.push(Item {
                source: path.clone(),
                output: PathBuf::new(),
                nozzle: String::new(),
            });
            continue;
        };
        if !["0.2", "0.25", "0.4", "0.6", "0.8"].contains(&nozzle) {
            return Err(format!(
                "invalid physical nozzle {nozzle:?} for batch source"
            ));
        }
        // Go's writeConversion chooses the first free name in the source's
        // own directory. Reserve earlier planned names within this batch.
        let destination =
            next_original_output_with(path, target, &destinations, |candidate| exists(candidate))?;
        if crate::same_path(path, &destination) {
            return Err("batch output cannot be the original source".into());
        }
        destinations.insert(output_key(&destination));
        result.push(Item {
            source: path.clone(),
            output: destination,
            nozzle: nozzle.to_owned(),
        });
    }
    Ok(result)
}

/// Resolve the same target independently for each 3MF source, mirroring
/// native/batch.go configuredConvertContext(source, target, summary.Nozzle, pref).
/// This is read-only. No worker is started and no output is opened here.
fn prepare_batch<F>(
    paths: &[PathBuf],
    target: &str,
    settings: &Value,
    resolver: F,
) -> Result<Vec<PreparedItem>, String>
where
    F: FnMut(&str, &str, &BTreeMap<String, String>) -> Result<Value, String>,
{
    prepare_batch_with(paths, target, settings, resolver, output_exists)
}

// A read-only injectable probe models file-name races and inaccessible
// destinations without creating any 3MF output during QA.
fn prepare_batch_with<F, E>(
    paths: &[PathBuf],
    target: &str,
    settings: &Value,
    mut resolver: F,
    mut exists: E,
) -> Result<Vec<PreparedItem>, String>
where
    F: FnMut(&str, &str, &BTreeMap<String, String>) -> Result<Value, String>,
    E: FnMut(&Path) -> Result<bool, String>,
{
    if paths.is_empty() || paths.len() > MAX_BATCH {
        return Err(format!("batch requires 1..={MAX_BATCH} source files"));
    }
    // Preserve the existing direct-API duplicate refusal. Errors belonging
    // only to one source/target are instead recorded per item below.
    let mut seen = BTreeSet::new();
    for path in paths {
        let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !seen.insert(output_key(&identity)) {
            return Err(format!("duplicate 3MF source: {}", path.display()));
        }
    }
    let mut metadata = Vec::with_capacity(paths.len());
    let mut variant_nozzles = Vec::with_capacity(paths.len());
    // The stable Go loop records a SOURCE_INVALID failure and processes the
    // next file. Keep a rejected source in place, without an OEM or output.
    let mut verified_bundles: BTreeMap<String, Result<Arc<Value>, String>> = BTreeMap::new();
    for path in paths {
        let inspected = inspect_source_nozzle(path).and_then(|source| {
            let source_nozzle = source["source_nozzle"]
                .as_str()
                .ok_or("source nozzle inspection returned no diameter")?
                .to_owned();
            Ok((source_nozzle, source["source_nozzle_assumed"] == true))
        });
        let (source_nozzle, assumed) = match inspected {
            Ok(details) => details,
            Err(err) => {
                variant_nozzles.push(None);
                metadata.push((
                    String::new(),
                    false,
                    true,
                    Err(format!("{}: {err}", path.display())),
                ));
                continue;
            }
        };
        let (target_nozzle, overrides) = match printer_settings::profile_choice_for_source(
            settings,
            target,
            "auto",
            &source_nozzle,
        ) {
            Ok(choice) => choice,
            Err(err) => {
                variant_nozzles.push(None);
                metadata.push((
                    source_nozzle,
                    assumed,
                    false,
                    Err(format!("{}: {err}", path.display())),
                ));
                continue;
            }
        };
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
        variant_nozzles.push(Some(target_nozzle));
        metadata.push((source_nozzle, assumed, false, resolved));
    }
    // Go's name/permission errors are per-file and per-target results.
    // A single saturated directory must not refuse the entire batch.
    let mut filenames = Vec::with_capacity(paths.len());
    for ((path, nozzle), meta) in paths.iter().zip(&variant_nozzles).zip(&mut metadata) {
        if meta.3.is_err() {
            filenames.push(Item {
                source: path.clone(),
                output: PathBuf::new(),
                nozzle: nozzle.clone().unwrap_or_default(),
            });
            continue;
        }
        match plan_batch_with(
            std::slice::from_ref(path),
            target,
            std::slice::from_ref(nozzle),
            |candidate| exists(candidate),
        ) {
            Ok(mut planned) => filenames.push(planned.remove(0)),
            Err(error) => {
                meta.3 = Err(error);
                filenames.push(Item {
                    source: path.clone(),
                    output: PathBuf::new(),
                    nozzle: nozzle.clone().unwrap_or_default(),
                });
            }
        }
    }
    Ok(filenames
        .into_iter()
        .zip(metadata)
        .map(
            |(item, (source_nozzle, source_nozzle_assumed, source_invalid, resolved))| {
                PreparedItem {
                    item,
                    source_nozzle,
                    source_nozzle_assumed,
                    source_invalid,
                    resolved,
                }
            },
        )
        .collect())
}

// Group the existing per-target OEM planning by source in original Go order.
// One bad target does not invalidate another target for the same source.
// No conversion worker is instantiated here.
fn prepare_multi_batch<F>(
    paths: &[PathBuf],
    targets: &[String],
    settings: &Value,
    mut resolver: F,
) -> Result<Vec<PreparedSource>, String>
where
    F: FnMut(&str, &str, &BTreeMap<String, String>) -> Result<Value, String>,
{
    if targets.is_empty() || targets.len() > 32 {
        return Err("select between 1 and 32 printers".into());
    }
    let mut seen = BTreeSet::new();
    for target in targets {
        if !seen.insert(target) {
            return Err(format!("invalid or repeated target {target:?}"));
        }
    }
    let mut sources = paths
        .iter()
        .map(|source| {
            // One source inspection supplies geometry + Go metadata summary.
            // No repeated large-model parse and no Registry lock held during I/O.
            let (source_facts, source_summary) = match inspect_source_details(source) {
                Ok((facts, summary)) => (Some(facts), Some(summary)),
                Err(_) => (None, None),
            };
            PreparedSource {
                source: source.clone(),
                source_facts,
                source_summary,
                targets: Vec::with_capacity(targets.len()),
            }
        })
        .collect::<Vec<_>>();
    for target in targets {
        let ready = prepare_batch(paths, target, settings, |t, n, overrides| {
            resolver(t, n, overrides)
        })?;
        for (source, item) in sources.iter_mut().zip(ready) {
            source.targets.push(PreparedTarget {
                target: target.clone(),
                ready: item,
            });
        }
    }
    Ok(sources)
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
                if phase == "completed" && detail["output"].as_str().is_some() {
                    batch["items"][index]["output"] = detail["output"].clone();
                }
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

// This is the only path that can reach the converter; rejected preflight
// entries have no output and are filtered out by run_batch_with.
fn execute_prepared(
    ready: &PreparedItem,
    engine: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, String> {
    let item = &ready.item;
    let bundle = ready.resolved.as_ref().map_err(Clone::clone)?;
    if item.output.as_os_str().is_empty() {
        return Err("rejected batch file has no output destination".into());
    }
    // Recheck source and materials after planning, before the worker starts.
    verify_source_nozzle_unchanged(ready)?;
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
    let target = bundle["target"]
        .as_str()
        .ok_or("resolved OEM is missing its printer target")?;
    // Go repeats the name lookup when a second writer wins the race. Our
    // worker still owns atomic, no-overwrite publication of the 3MF.
    let (_, receipt) = write_first_available_with(&item.source, target, output_exists, |output| {
        if cancel.load(Ordering::SeqCst) {
            return Err(OutputAttemptError::Other(
                "batch cancelled before worker start".into(),
            ));
        }
        let start = worker.execute(
            "conversion.start",
            &json!({"output":output,"engine":engine}),
        );
        if start["ok"] != true {
            let error = start["error"].as_str().unwrap_or("unknown");
            if error == "destination exists; will not overwrite" && output.exists() {
                return Err(OutputAttemptError::Exists);
            }
            return Err(OutputAttemptError::Other(format!(
                "worker failed to start: {error}"
            )));
        }
        loop {
            if cancel.load(Ordering::SeqCst) {
                worker.execute("conversion.cancel", &json!({}));
            }
            let status = worker.snapshot();
            match status["phase"].as_str().unwrap_or("unknown") {
                "running" => thread::sleep(Duration::from_millis(80)),
                "completed" => {
                    let converted = status["output"].as_str().ok_or_else(|| {
                        OutputAttemptError::Other("conversion completed without output file".into())
                    })?;
                    if converted != output.to_string_lossy() {
                        return Err(OutputAttemptError::Other(
                            "batch worker output path differs from planned path".into(),
                        ));
                    }
                    if !output.is_file() {
                        return Err(OutputAttemptError::Other(
                            "batch worker output file missing".into(),
                        ));
                    }
                    let size = fs::metadata(output)
                        .map_err(|e| OutputAttemptError::Other(e.to_string()))?
                        .len();
                    return Ok(json!({
                        "validation": validation,
                        "source_nozzle":ready.source_nozzle,
                        "source_nozzle_assumed":ready.source_nozzle_assumed,
                        "target_nozzle":item.nozzle,
                        "slicer_id":bundle["slicer_id"],
                        "native_slicer":bundle["slicer"],
                        "conversion":status["result"],
                        "output":output,
                        "bytes":size
                    }));
                }
                "failed" if is_worker_output_exists(&status["result"], output.exists()) => {
                    return Err(OutputAttemptError::Exists);
                }
                "failed" | "cancelled" => {
                    return Err(OutputAttemptError::Other(format!(
                        "conversion {}: {}",
                        status["phase"], status["result"]
                    )));
                }
                phase => {
                    return Err(OutputAttemptError::Other(format!(
                        "unexpected conversion phase: {phase}"
                    )))
                }
            }
        }
    })?;
    Ok(receipt)
}

// The injectable executor lets QA prove mixed failure/success progress without
// any real conversion, slicing, G-code generation or printer operation.
fn run_batch_with<F>(
    shared: Arc<Mutex<Session>>,
    items: Vec<PreparedItem>,
    cancel: Arc<AtomicBool>,
    mut execute: F,
) where
    F: FnMut(&PreparedItem) -> Result<Value, String>,
{
    for (index, ready) in items.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            finish(&shared, "cancelled", Some("batch cancelled between items"));
            return;
        }
        update(&shared, index, "running", None);
        let result = ready
            .resolved
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|_| execute(ready));
        match result {
            Ok(receipt) => update(&shared, index, "completed", Some(receipt)),
            Err(error) => {
                if cancel.load(Ordering::SeqCst) {
                    update(&shared, index, "cancelled", Some(json!({"error":error})));
                    finish(&shared, "cancelled", Some(&error));
                    return;
                }
                update(
                    &shared,
                    index,
                    "failed",
                    Some(failure_receipt(&error, ready.source_invalid)),
                );
            }
        }
    }
    finish(&shared, "completed", None);
}

fn run_batch(
    shared: Arc<Mutex<Session>>,
    items: Vec<PreparedItem>,
    engine: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    let worker_cancel = Arc::clone(&cancel);
    run_batch_with(shared, items, cancel, move |ready| {
        execute_prepared(ready, &engine, &worker_cancel)
    });
}

// Nested file and target receipts preserve Go's per-file Results/Failures,
// while keeping the existing GPUI batch progress fields.
fn multi_batch_initial(sources: &[PreparedSource], targets: &[String]) -> Value {
    json!({
        "phase":"running", "targets":targets,
        "total":sources.len(), "completed":0, "filesDone":0,
        "successes":0, "failures":0, "progress_percent":0,
        "active_index":null, "error":null,
        "items":sources.iter().map(|source| json!({
            "source":source.source, "phase":"queued", "error":null,
            // Null when the source is invalid; never invent a successful inspection.
            "source_facts":source.source_facts.as_ref().map(|facts|json!({
                "objects":facts.objects, "instances":facts.instances
            })),
            "source_summary":source.source_summary,
            "results":[], "failures":[],
            "targets":source.targets.iter().map(|pair| json!({
                "target":pair.target,
                "source":pair.ready.item.source,
                "output":if pair.ready.item.output.as_os_str().is_empty() {
                    Value::Null
                } else {
                    json!(pair.ready.item.output)
                },
                "nozzle":pair.ready.item.nozzle,
                "source_nozzle":pair.ready.source_nozzle,
                "source_nozzle_assumed":pair.ready.source_nozzle_assumed,
                "phase":"queued", "receipt":null
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

// The stable Go BatchReply increments FilesDone once per source and reports
// Results/Failures independently for every requested target. This keeps the
// same sequence and the source-first result shape for Rust.
fn mark_multi_target(
    shared: &Arc<Mutex<Session>>,
    file_index: usize,
    target_index: usize,
    phase: &str,
    receipt: Value,
) {
    if let Ok(mut session) = shared.lock() {
        if let Some(batch) = session.batch.as_mut() {
            let target = batch["items"][file_index]["targets"][target_index]["target"].clone();
            batch["items"][file_index]["targets"][target_index]["phase"] = json!(phase);
            if phase == "completed" && receipt["output"].as_str().is_some() {
                batch["items"][file_index]["targets"][target_index]["output"] =
                    receipt["output"].clone();
            }
            batch["items"][file_index]["targets"][target_index]["receipt"] = receipt.clone();
            if phase == "completed" || phase == "failed" {
                let counter = if phase == "completed" {
                    "successes"
                } else {
                    "failures"
                };
                batch[counter] = json!(batch[counter].as_u64().unwrap_or(0) + 1);
                let list = if phase == "completed" {
                    "results"
                } else {
                    "failures"
                };
                if let Some(results) = batch["items"][file_index][list].as_array_mut() {
                    results.push(json!({"target":target,"receipt":receipt}));
                }
                if receipt["code"] == "SOURCE_INVALID" {
                    batch["items"][file_index]["error"] = receipt["error"].clone();
                }
            }
        }
    }
}

fn complete_multi_file(shared: &Arc<Mutex<Session>>, index: usize) {
    if let Ok(mut session) = shared.lock() {
        if let Some(batch) = session.batch.as_mut() {
            let failed = batch["items"][index]["targets"]
                .as_array()
                .is_some_and(|targets| targets.iter().any(|target| target["phase"] == "failed"));
            batch["items"][index]["phase"] = json!(if failed { "failed" } else { "completed" });
            let completed = batch["completed"].as_u64().unwrap_or(0) + 1;
            batch["completed"] = json!(completed);
            batch["filesDone"] = json!(completed);
            batch["progress_percent"] = json!((completed * 100)
                .checked_div(batch["total"].as_u64().unwrap_or(0))
                .unwrap_or(0));
            batch["active_index"] = Value::Null;
        }
    }
}

fn finish_multi(shared: &Arc<Mutex<Session>>, phase: &str, error: Option<&str>) {
    finish(shared, phase, error);
    if phase == "cancelled" || phase == "failed" {
        if let Ok(mut session) = shared.lock() {
            if let Some(batch) = session.batch.as_mut() {
                if let Some(files) = batch["items"].as_array_mut() {
                    for file in files {
                        if let Some(targets) = file["targets"].as_array_mut() {
                            for target in targets {
                                if target["phase"] == "queued" {
                                    target["phase"] = json!("skipped");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// Four concurrent target workers at most, with a strict source-to-source
// barrier. The injected callback in tests is a pure mock, never a converter.
fn run_multi_batch_with<F>(
    shared: Arc<Mutex<Session>>,
    sources: Vec<PreparedSource>,
    cancel: Arc<AtomicBool>,
    execute: F,
) where
    F: Fn(&PreparedItem) -> Result<Value, String> + Sync,
{
    for (file_index, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            finish_multi(
                &shared,
                "cancelled",
                Some("batch cancelled between sources"),
            );
            return;
        }
        if let Ok(mut session) = shared.lock() {
            if let Some(batch) = session.batch.as_mut() {
                batch["active_index"] = json!(file_index);
                batch["items"][file_index]["phase"] = json!("running");
            }
        }
        for (chunk_index, chunk) in source.targets.chunks(4).enumerate() {
            if cancel.load(Ordering::SeqCst) {
                finish_multi(
                    &shared,
                    "cancelled",
                    Some("batch cancelled between target groups"),
                );
                return;
            }
            let callback = &execute;
            let results: Vec<Result<Value, String>> = thread::scope(|scope| {
                let handles = chunk
                    .iter()
                    .map(|pair| {
                        scope.spawn(move || {
                            pair.ready
                                .resolved
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|_| callback(&pair.ready))
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| match handle.join() {
                        Ok(result) => result,
                        Err(_) => Err("batch target worker panicked".into()),
                    })
                    .collect()
            });
            for (index_in_chunk, (pair, result)) in chunk.iter().zip(results).enumerate() {
                let index = chunk_index * 4 + index_in_chunk;
                match result {
                    Ok(receipt) => {
                        mark_multi_target(&shared, file_index, index, "completed", receipt)
                    }
                    Err(error) if cancel.load(Ordering::SeqCst) => {
                        mark_multi_target(
                            &shared,
                            file_index,
                            index,
                            "cancelled",
                            json!({"error":error}),
                        );
                    }
                    Err(error) => {
                        mark_multi_target(
                            &shared,
                            file_index,
                            index,
                            "failed",
                            failure_receipt(&error, pair.ready.source_invalid),
                        );
                    }
                }
            }
            if cancel.load(Ordering::SeqCst) {
                finish_multi(
                    &shared,
                    "cancelled",
                    Some("batch cancelled during target group"),
                );
                return;
            }
        }
        complete_multi_file(&shared, file_index);
    }
    finish_multi(&shared, "completed", None);
}

fn run_multi_batch(
    shared: Arc<Mutex<Session>>,
    sources: Vec<PreparedSource>,
    engine: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    let worker_cancel = Arc::clone(&cancel);
    run_multi_batch_with(shared, sources, cancel, move |ready| {
        execute_prepared(ready, &engine, &worker_cancel)
    });
}

impl Registry {
    fn batch_start_multiple(
        &self,
        params: &Value,
        sources: Vec<PathBuf>,
        targets: Vec<String>,
        engine: PathBuf,
    ) -> Result<Value, String> {
        // The selection comes from the same isolated My Printers inventory
        // displayed by GPUI. Catalogue membership never grants OEM readiness.
        let saved = crate::my_printers::read(
            &crate::my_printers::experimental_path()?,
            &crate::my_printers::available_installed_ids(),
        )?;
        let selected = saved["selectedIds"]
            .as_array()
            .ok_or("invalid My Printers selection")?
            .iter()
            .map(|id| id.as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        validate_multi_targets(&targets, &selected)?;
        {
            let session = self.state.lock().map_err(|e| e.to_string())?;
            if busy(&session) {
                return Err("cannot start batch while conversion or batch is active".into());
            }
        }
        let settings = printer_settings::current()?;
        let items = prepare_multi_batch(
            &sources,
            &targets,
            &settings,
            |target, nozzle, overrides| {
                let slicer = settings["printers"][target]["slicer"]
                    .as_str()
                    .unwrap_or("auto");
                oem::resolve_configured(target, nozzle, slicer, overrides)
            },
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut session = self.state.lock().map_err(|e| e.to_string())?;
            if busy(&session) {
                return Err("another conversion started during batch preparation".into());
            }
            if printer_settings::current()? != settings {
                return Err("printer settings changed during batch profile resolution".into());
            }
            if params.get("paths").is_none() && session.batch_staged != sources {
                return Err("staged batch sources changed during profile resolution".into());
            }
            session.batch = Some(multi_batch_initial(&items, &targets));
            session.batch_cancel = Some(cancel.clone());
        }
        let count = items.len();
        let shared = self.state.clone();
        thread::spawn(move || run_multi_batch(shared, items, engine, cancel));
        Ok(json!({"started":true, "phase":"running", "count":count, "targets":targets}))
    }

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
        state.source_revision = state.source_revision.wrapping_add(1);
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

    // Source-locked native/frontend/dist/app.js::renderQueue() remove action.
    // It modifies only queue memory and selected-source references, never 3MF
    // bytes or the existing completed batch receipt.
    pub(crate) fn batch_remove(&self, params: &Value) -> Result<Value, String> {
        let path = PathBuf::from(arg(params, "path")?);
        let mut state = self.state.lock().map_err(|err| err.to_string())?;
        if busy(&state) {
            return Err("cannot remove source while conversion is running".into());
        }
        let from_queue = state
            .batch_staged
            .iter()
            .position(|entry| entry == &path || same_path(entry, &path));
        let selected = state
            .source
            .as_ref()
            .is_some_and(|source| source == &path || same_path(source, &path));
        if from_queue.is_none() && !selected {
            return Err("source is not in the queue".into());
        }
        if let Some(index) = from_queue {
            state.batch_staged.remove(index);
        }
        if selected {
            state.source = None;
            state.source_revision = state.source_revision.wrapping_add(1);
            state.facts = None;
            state.bundle_path = None;
            state.bundle_label = None;
            state.bundle = None;
            state.installed_oem = false;
            state.target = None;
            state.nozzle = None;
            state.selected_materials.clear();
            state.output = None;
            // Historical result receipts remain readable, while no source
            // from a deleted queue entry remains selected for a future job.
        }
        Ok(json!({
            "removed":true,
            "staged":state.batch_staged.len(),
            "paths":state.batch_staged,
        }))
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
        let sources = {
            let session = self.state.lock().map_err(|e| e.to_string())?;
            if params.get("paths").is_some() {
                parse_sources(params)?
            } else {
                session.batch_staged.clone()
            }
        };
        let engine = engine_path(params.get("engine").and_then(Value::as_str))?;
        if !engine.is_file() {
            return Err(format!("batch Rust engine missing: {}", engine.display()));
        }
        // Explicit printer targets use Go's source-first, multi-target model.
        // Legacy single-target callers retain the previous selected OEM path.
        if let Some(targets) = parse_targets(params)? {
            return self.batch_start_multiple(params, sources, targets, engine);
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
        let items = prepare_batch(
            &sources,
            &target,
            &settings,
            |printer, nozzle, overrides| {
                let slicer = settings["printers"][printer]["slicer"]
                    .as_str()
                    .unwrap_or("auto");
                oem::resolve_configured(printer, nozzle, slicer, overrides)
            },
        )?;
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
            "target":target,"nozzle":batch_nozzle}))
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
        assert_eq!(
            ready[1].item.output.file_name().unwrap(),
            "U1-bambu-nozzle-0.6.3mf"
        );
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
        let ready = prepare_batch(&paths, "u1", &settings, |target, nozzle, _| {
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
        let error = prepare_batch(&missing, "u1", &printer_settings::defaults(), |_, _, _| {
            panic!("oversized batch must not inspect/resolve files")
        })
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
        let failed = prepare_batch(&paths, "u1", &settings, |target, nozzle, _| {
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
        let mismatch = prepare_batch(&paths[..1], "u1", &settings, |target, _, _| {
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
        let rejected = prepare_batch(
            std::slice::from_ref(&invalid),
            "u1",
            &settings,
            |_, _, _| panic!("an invalid source must never resolve an OEM"),
        )
        .unwrap();
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].resolved.is_err());
        assert!(rejected[0].item.output.as_os_str().is_empty());
        assert!(rejected[0].item.nozzle.is_empty());
        assert!(!root.join("not-a-3mf-slicebridge-u1-0_4.3mf").exists());
        assert_eq!(fs::read(&invalid).unwrap(), b"invalid source");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupted_original_3mf_between_valid_files_does_not_abort_or_execute() {
        let root = output_folder("corrupt-middle");
        let valid_before = fixture("bambu-nozzle-0.2.3mf");
        let valid_after = fixture("bambu-nozzle-0.6.3mf");
        let original_before = fs::read(&valid_before).unwrap();
        let original_after = fs::read(&valid_after).unwrap();
        let corrupted = root.join("damaged.3mf");
        // A truncated copy of a real 3MF, not an invented extension or model.
        fs::write(
            &corrupted,
            &original_before[..original_before.len().min(64)],
        )
        .unwrap();
        let paths = [valid_before.clone(), corrupted.clone(), valid_after.clone()];
        let mut resolved = Vec::new();
        let ready = prepare_batch(
            &paths,
            "u1",
            &printer_settings::defaults(),
            |target, nozzle, _| {
                resolved.push(nozzle.to_owned());
                Ok(json!({"target":target,"nozzle":nozzle}))
            },
        )
        .unwrap();
        assert_eq!(resolved, vec!["0.2", "0.6"]);
        assert_eq!(ready.len(), 3);
        assert!(ready[0].resolved.is_ok());
        assert!(ready[1].resolved.is_err());
        assert!(ready[1].source_invalid);
        assert!(ready[1].item.output.as_os_str().is_empty());
        assert!(ready[2].resolved.is_ok());
        assert!(!ready[0].item.output.exists());
        assert!(!ready[2].item.output.exists());

        let shared = Arc::new(Mutex::new(Session::default()));
        {
            let mut state = shared.lock().unwrap();
            state.batch = Some(json!({
                "phase":"running","total":3,"completed":0,"successes":0,
                "failures":0,"progress_percent":0,
                "items":[
                    {"phase":"queued","receipt":null},
                    {"phase":"queued","receipt":null},
                    {"phase":"queued","receipt":null}
                ]
            }));
        }
        let mut executed_sources = Vec::new();
        run_batch_with(
            shared.clone(),
            ready,
            Arc::new(AtomicBool::new(false)),
            |entry| {
                // No real engine can run: this is a synthetic receipt for QA.
                assert!(!entry.item.output.as_os_str().is_empty());
                executed_sources.push(entry.item.source.clone());
                Ok(json!({"mocked":true}))
            },
        );
        assert_eq!(
            executed_sources,
            vec![valid_before.clone(), valid_after.clone()]
        );
        let state = shared.lock().unwrap();
        let batch = state.batch.as_ref().unwrap();
        assert_eq!(batch["phase"], "completed");
        assert_eq!(batch["completed"], 3);
        assert_eq!(batch["successes"], 2);
        assert_eq!(batch["failures"], 1);
        assert_eq!(batch["progress_percent"], 100);
        assert_eq!(batch["items"][0]["phase"], "completed");
        assert_eq!(batch["items"][1]["phase"], "failed");
        assert_eq!(batch["items"][1]["receipt"]["code"], "SOURCE_INVALID");
        assert!(batch["items"][1]["receipt"]["error"]
            .as_str()
            .unwrap()
            .contains("damaged.3mf"));
        assert_eq!(batch["items"][2]["phase"], "completed");
        drop(state);
        assert_eq!(fs::read(valid_before).unwrap(), original_before);
        assert_eq!(fs::read(valid_after).unwrap(), original_after);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1); // corrupt source only
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_multi_targets_require_unique_saved_my_printers_inventory() {
        let saved = vec!["snapmaker-u1".into(), "qidi-x-plus-5".into()];
        let targets = vec!["u1".into(), "qidi".into()];
        assert!(validate_multi_targets(&targets, &saved).is_ok());
        assert!(validate_multi_targets(&["c5".into()], &saved).is_err());
        assert!(validate_multi_targets(&["orca-opaque".into()], &saved).is_err());
        assert!(validate_multi_targets(&["u1".into(), "u1".into()], &saved).is_err());
        assert!(validate_multi_targets(&[], &saved).is_err());
        assert!(validate_multi_targets(&vec!["u1".into(); 33], &saved).is_err());
        assert_eq!(parse_targets(&json!({})).unwrap(), None);
        assert_eq!(
            parse_targets(&json!({"targets":["u1","qidi"]})).unwrap(),
            Some(targets)
        );
        assert!(parse_targets(&json!({"targets":["u1","u1"]})).is_err());
        assert!(parse_targets(&json!({"targets":["u1",42]})).is_err());
        assert!(parse_targets(&json!({"targets":[]})).is_err());
    }

    #[test]
    fn multi_targets_have_source_first_receipts_and_continue_after_bad_original_3mf() {
        use std::sync::atomic::AtomicUsize;
        let root = output_folder("multi-target-no-conversion");
        let valid_a = fixture("bambu-nozzle-0.2.3mf");
        let valid_b = fixture("bambu-nozzle-0.6.3mf");
        let before_a = fs::read(&valid_a).unwrap();
        let before_b = fs::read(&valid_b).unwrap();
        let corrupted = root.join("corrupted.3mf");
        fs::write(&corrupted, &before_a[..before_a.len().min(27)]).unwrap();
        let paths = vec![valid_a.clone(), corrupted.clone(), valid_b.clone()];
        let targets = vec!["u1".to_owned(), "qidi".to_owned()];
        let mut resolutions = Vec::new();
        let sources = prepare_multi_batch(
            &paths,
            &targets,
            &printer_settings::defaults(),
            |target, nozzle, _| {
                resolutions.push((target.to_owned(), nozzle.to_owned()));
                Ok(json!({"target":target,"nozzle":nozzle}))
            },
        )
        .unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0].source, valid_a);
        assert_eq!(sources[1].source, corrupted);
        assert_eq!(sources[2].source, valid_b);
        assert_eq!(sources[0].source_facts.as_ref().unwrap().objects, 1);
        assert_eq!(sources[1].source_facts, None);
        assert_eq!(sources[2].source_facts.as_ref().unwrap().objects, 1);
        let pending = multi_batch_initial(&sources, &targets);
        assert_eq!(pending["items"][0]["source_facts"]["objects"], 1);
        assert_eq!(pending["items"][0]["source_facts"]["instances"], 1);
        assert_eq!(pending["items"][0]["source_summary"]["plates"], 1);
        assert_eq!(
            pending["items"][0]["source_summary"]["filaments"][0]["type"],
            "PLA"
        );
        assert!(pending["items"][1]["source_facts"].is_null());
        assert!(pending["items"][1]["source_summary"].is_null());
        assert_eq!(pending["items"][2]["source_facts"]["objects"], 1);
        assert_eq!(resolutions.len(), 4); // invalid source resolved for neither target
        assert!(sources[1].targets.iter().all(|t| t.ready.source_invalid
            && t.ready.resolved.is_err()
            && t.ready.item.output.as_os_str().is_empty()));
        assert_eq!(
            sources[0]
                .targets
                .iter()
                .map(|t| t.target.as_str())
                .collect::<Vec<_>>(),
            vec!["u1", "qidi"]
        );
        assert!(sources[0].targets[0]
            .ready
            .item
            .output
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("U1-"));
        assert!(sources[0].targets[1]
            .ready
            .item
            .output
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("XPlus5-"));
        let shared = Arc::new(Mutex::new(Session::default()));
        shared.lock().unwrap().batch = Some(multi_batch_initial(&sources, &targets));
        let mock_calls = AtomicUsize::new(0);
        run_multi_batch_with(
            shared.clone(),
            sources,
            Arc::new(AtomicBool::new(false)),
            |ready| {
                mock_calls.fetch_add(1, Ordering::SeqCst);
                assert!(!ready.item.output.exists());
                let output = ready.item.output.file_name().unwrap().to_string_lossy();
                if output.starts_with("XPlus5-") {
                    Err("fixture refused target".into())
                } else {
                    Ok(json!({"mocked":true}))
                }
            },
        );
        assert_eq!(mock_calls.load(Ordering::SeqCst), 4);
        let state = shared.lock().unwrap();
        let batch = state.batch.as_ref().unwrap();
        assert_eq!(batch["phase"], "completed");
        assert_eq!(batch["completed"], 3);
        assert_eq!(batch["filesDone"], 3);
        assert_eq!(batch["successes"], 2);
        assert_eq!(batch["failures"], 4);
        assert_eq!(batch["progress_percent"], 100);
        assert_eq!(batch["items"][0]["phase"], "failed");
        assert_eq!(batch["items"][0]["results"].as_array().unwrap().len(), 1);
        assert_eq!(batch["items"][0]["failures"].as_array().unwrap().len(), 1);
        assert_eq!(batch["items"][0]["results"][0]["target"], "u1");
        assert_eq!(batch["items"][0]["failures"][0]["target"], "qidi");
        assert_eq!(batch["items"][1]["failures"].as_array().unwrap().len(), 2);
        assert_eq!(
            batch["items"][1]["targets"][0]["receipt"]["code"],
            "SOURCE_INVALID"
        );
        assert_eq!(
            batch["items"][1]["targets"][1]["receipt"]["code"],
            "SOURCE_INVALID"
        );
        assert_eq!(batch["items"][2]["results"][0]["target"], "u1");
        assert_eq!(batch["items"][2]["failures"][0]["target"], "qidi");
        drop(state);
        assert_eq!(fs::read(valid_a).unwrap(), before_a);
        assert_eq!(fs::read(valid_b).unwrap(), before_b);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1); // corrupt source only
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn four_target_workers_max_with_source_barrier_and_mock_only() {
        use std::sync::atomic::AtomicUsize;
        let root = output_folder("parallel-mock");
        let one = fixture("bambu-nozzle-0.2.3mf");
        let ready = prepare_batch(
            std::slice::from_ref(&one),
            "u1",
            &printer_settings::defaults(),
            |target, nozzle, _| Ok(json!({"target":target,"nozzle":nozzle})),
        )
        .unwrap()
        .remove(0);
        let sources = vec![PreparedSource {
            source_facts: slicebridge_3mf_probe::inspect_package(&one).ok(),
            source_summary: slicebridge_3mf_probe::inspect_source_summary(&one).ok(),
            source: one,
            targets: (0..8)
                .map(|index| PreparedTarget {
                    target: format!("fixture-{index}"),
                    ready: ready.clone(),
                })
                .collect(),
        }];
        let targets = (0..8).map(|i| format!("fixture-{i}")).collect::<Vec<_>>();
        let shared = Arc::new(Mutex::new(Session::default()));
        shared.lock().unwrap().batch = Some(multi_batch_initial(&sources, &targets));
        let active = AtomicUsize::new(0);
        let max_active = AtomicUsize::new(0);
        run_multi_batch_with(
            shared.clone(),
            sources,
            Arc::new(AtomicBool::new(false)),
            |_| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(json!({"mocked":true}))
            },
        );
        assert_eq!(max_active.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        let state = shared.lock().unwrap();
        assert_eq!(state.batch.as_ref().unwrap()["completed"], 1);
        assert_eq!(state.batch.as_ref().unwrap()["successes"], 8);
        assert_eq!(state.batch.as_ref().unwrap()["failures"], 0);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
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
    fn exhausted_or_inaccessible_output_for_one_source_does_not_refuse_batch() {
        let paths = [
            fixture("bambu-nozzle-0.2.3mf"),
            fixture("bambu-nozzle-0.6.3mf"),
        ];
        let before = paths
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>();
        let settings = printer_settings::defaults();
        let resolved = |target: &str, nozzle: &str, _: &BTreeMap<String, String>| {
            Ok(json!({"target":target,"nozzle":nozzle}))
        };
        let planned = prepare_batch_with(&paths, "u1", &settings, resolved, |candidate| {
            Ok(candidate
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("U1-bambu-nozzle-0.2"))
        })
        .unwrap();
        assert_eq!(planned.len(), 2);
        assert!(planned[0]
            .resolved
            .as_ref()
            .unwrap_err()
            .contains("OUTPUT_COLLISIONS"));
        assert!(planned[0].item.output.as_os_str().is_empty());
        assert!(planned[1].resolved.is_ok());
        assert!(!planned[1].item.output.exists());

        let shared = Arc::new(Mutex::new(Session::default()));
        shared.lock().unwrap().batch = Some(json!({
            "phase":"running","total":2,"completed":0,"successes":0,
            "failures":0,"progress_percent":0,
            "items":[{"phase":"queued","receipt":null},
                     {"phase":"queued","receipt":null}]
        }));
        let mut mock_calls = Vec::new();
        run_batch_with(
            shared.clone(),
            planned,
            Arc::new(AtomicBool::new(false)),
            |entry| {
                mock_calls.push(entry.item.source.clone());
                Ok(json!({"mock":true}))
            },
        );
        assert_eq!(mock_calls, vec![paths[1].clone()]);
        let state = shared.lock().unwrap();
        let batch = state.batch.as_ref().unwrap();
        assert_eq!(batch["phase"], "completed");
        assert_eq!(batch["completed"], 2);
        assert_eq!(batch["successes"], 1);
        assert_eq!(batch["failures"], 1);
        assert_eq!(batch["items"][0]["receipt"]["code"], "OUTPUT_COLLISIONS");
        assert_eq!(
            batch["items"][0]["receipt"]["error"],
            "Too many same-name outputs"
        );
        assert_eq!(batch["items"][1]["phase"], "completed");
        drop(state);

        let permission = prepare_batch_with(
            &paths,
            "u1",
            &settings,
            |target, nozzle, _| Ok(json!({"target":target,"nozzle":nozzle})),
            |candidate| {
                if candidate
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("U1-bambu-nozzle-0.2")
                {
                    Err("OUTPUT_INVALID: permission denied".to_owned())
                } else {
                    Ok(false)
                }
            },
        )
        .unwrap();
        assert!(permission[0].resolved.is_err());
        assert!(permission[0].item.output.as_os_str().is_empty());
        assert!(permission[1].resolved.is_ok());
        assert_eq!(
            failure_receipt(permission[0].resolved.as_ref().unwrap_err(), false),
            json!({"code":"OUTPUT_INVALID","error":"permission denied"})
        );
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn original_output_name_follows_main_for_all_builtin_printers() {
        let src = Path::new("/read-only/models/complex.name.3MF");
        for (target, prefix) in [
            ("u1", "U1"),
            ("c5", "Creator5"),
            ("cc2", "CentauriCarbon2"),
            ("qidi", "XPlus5"),
        ] {
            assert_eq!(
                original_output_name(src, target, 1).unwrap(),
                Path::new("/read-only/models").join(format!("{prefix}-complex.name.3MF"))
            );
            assert_eq!(
                original_output_name(src, target, 2).unwrap(),
                Path::new("/read-only/models").join(format!("{prefix}-complex.name-2.3MF"))
            );
            assert_eq!(
                original_output_name(src, target, 100).unwrap(),
                Path::new("/read-only/models").join(format!("{prefix}-complex.name-100.3MF"))
            );
        }
        assert!(original_output_name(src, "not-ported", 1).is_err());
    }

    #[test]
    fn output_name_retries_only_actual_collision_even_after_preflight() {
        let src = Path::new("/read-only/design.3mf");
        // Initial existence probe says 'free'; competing writer takes it
        // between the probe and converter. The fake converter writes nothing.
        let mut invoked = Vec::new();
        let (out, data) = write_first_available_with(
            src,
            "qidi",
            |_| Ok(false),
            |path| {
                invoked.push(path.to_path_buf());
                if invoked.len() == 1 {
                    Err(OutputAttemptError::Exists)
                } else {
                    Ok(json!({"mock":true, "output":path}))
                }
            },
        )
        .unwrap();
        assert_eq!(
            invoked,
            [
                PathBuf::from("/read-only/XPlus5-design.3mf"),
                PathBuf::from("/read-only/XPlus5-design-2.3mf"),
            ]
        );
        assert_eq!(out, invoked[1]);
        assert_eq!(data["output"], out.to_string_lossy().as_ref());

        // The first candidate is already present; a race takes the second.
        let mut calls = Vec::new();
        let (out, _) = write_first_available_with(
            src,
            "u1",
            |candidate| Ok(candidate.file_name().unwrap() == "U1-design.3mf"),
            |candidate| {
                calls.push(candidate.to_path_buf());
                if calls.len() == 1 {
                    Err(OutputAttemptError::Exists)
                } else {
                    Ok(json!({"mock":true}))
                }
            },
        )
        .unwrap();
        assert_eq!(
            calls,
            [
                PathBuf::from("/read-only/U1-design-2.3mf"),
                PathBuf::from("/read-only/U1-design-3.3mf"),
            ]
        );
        assert_eq!(out, calls[1]);
    }

    #[test]
    fn collision_exhaustion_and_noncollision_errors_do_not_start_more_work() {
        let source = Path::new("/read-only/fixture.3mf");
        let mut retries = 0;
        let err = write_first_available_with(
            source,
            "u1",
            |_| Ok(false),
            |_| {
                retries += 1;
                Err(OutputAttemptError::Exists)
            },
        )
        .unwrap_err();
        assert_eq!(retries, 100);
        assert_eq!(err, "OUTPUT_COLLISIONS: Too many same-name outputs");

        let mut calls = 0;
        let err = write_first_available_with(
            source,
            "u1",
            |_| Ok(false),
            |_| {
                calls += 1;
                Err(OutputAttemptError::Other(
                    "PROFILE_INVALID: bad material".into(),
                ))
            },
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(err, "PROFILE_INVALID: bad material");

        let mut attempts = 0;
        let err = write_first_available_with(
            source,
            "u1",
            |_| Err("OUTPUT_INVALID: access denied".into()),
            |_| {
                attempts += 1;
                Ok(json!({"mock":true}))
            },
        )
        .unwrap_err();
        assert_eq!(attempts, 0);
        assert_eq!(err, "OUTPUT_INVALID: access denied");
    }

    #[test]
    fn rust_worker_collision_recognition_requires_known_error_and_existing_path() {
        let cases = [
            "destination already exists; refusing overwrite",
            "destination already exists; refusing to overwrite",
            "destination not created (no overwrite): File exists (os error 17)",
        ];
        for error in cases {
            let receipt = json!({"engine":{"ok":false,"error":error}});
            assert!(is_worker_output_exists(&receipt, true));
            assert!(!is_worker_output_exists(&receipt, false));
        }
        assert!(!is_worker_output_exists(
            &json!({"engine":{"error":"permission denied"}}),
            true
        ));
        assert!(!is_worker_output_exists(
            &json!({"engine":{"error":"destination not created (no overwrite): Permission denied (os error 13)"}}),
            true
        ));
        assert!(!is_worker_output_exists(
            &json!({"engine":{"error":"failed to open source"}}),
            true
        ));
        assert!(!is_worker_output_exists(
            &json!({"error":"destination already exists; refusing overwrite"}),
            true
        ));
    }

    #[test]
    fn batch_receipt_uses_actual_chosen_output_after_late_name_collision() {
        let shared = Arc::new(Mutex::new(Session::default()));
        shared.lock().unwrap().batch = Some(json!({
            "phase":"running", "total":1, "completed":0, "successes":0,
            "failures":0, "items":[{"phase":"queued", "output":"old-path.3mf", "receipt":null}]
        }));
        update(
            &shared,
            0,
            "completed",
            Some(json!({
                "output":"/read-only/U1-new-path-2.3mf","mock":true
            })),
        );
        let state = shared.lock().unwrap();
        assert_eq!(
            state.batch.as_ref().unwrap()["items"][0]["output"],
            "/read-only/U1-new-path-2.3mf"
        );
        drop(state);
        let shared = Arc::new(Mutex::new(Session::default()));
        shared.lock().unwrap().batch = Some(json!({
            "phase":"running", "successes":0,"failures":0,
            "items":[{"targets":[{"target":"u1","phase":"queued",
                "output":"old-path.3mf","receipt":null}],"results":[],"failures":[]}]
        }));
        mark_multi_target(
            &shared,
            0,
            0,
            "completed",
            json!({
                "output":"/read-only/U1-new-path-2.3mf","mock":true
            }),
        );
        let state = shared.lock().unwrap();
        assert_eq!(
            state.batch.as_ref().unwrap()["items"][0]["targets"][0]["output"],
            "/read-only/U1-new-path-2.3mf"
        );
    }

    #[test]
    fn filenames_are_deterministic_and_never_overwritten() {
        let base = std::env::temp_dir().join(format!("sb-batch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let source = base.join("design.3mf");
        fs::write(&source, b"not-a-real-3mf").unwrap();
        let items =
            plan_batch(std::slice::from_ref(&source), "qidi", &[Some("0.6".into())]).unwrap();
        let output = base.join("XPlus5-design.3mf");
        assert_eq!(items[0].output, output);
        assert!(plan_batch(
            &[source.clone(), source.clone()],
            "qidi",
            &[Some("0.6".into()), Some("0.6".into())]
        )
        .is_err());
        // Simulate existing outputs without creating conversion-output files.
        let existing = [output.clone(), base.join("XPlus5-design-2.3mf")];
        let chosen = next_original_output_with(&source, "qidi", &BTreeSet::new(), |path| {
            Ok(existing.iter().any(|candidate| candidate == path))
        })
        .unwrap();
        assert_eq!(chosen, base.join("XPlus5-design-3.3mf"));
        let mut reserved = BTreeSet::new();
        reserved.insert(output_key(&output));
        let second = next_original_output_with(&source, "qidi", &reserved, |_| Ok(false)).unwrap();
        assert_eq!(second, base.join("XPlus5-design-2.3mf"));
        let saturated =
            next_original_output_with(&source, "qidi", &BTreeSet::new(), |_| Ok(true)).unwrap_err();
        assert!(saturated.contains("OUTPUT_COLLISIONS"));
        let denied = next_original_output_with(&source, "qidi", &BTreeSet::new(), |_| {
            Err("OUTPUT_INVALID: permission denied".to_owned())
        })
        .unwrap_err();
        assert!(denied.contains("OUTPUT_INVALID"));
        assert_eq!(fs::read_dir(&base).unwrap().count(), 1); // source only
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn same_stem_in_different_source_folders_gets_original_sibling_outputs() {
        let root = std::env::temp_dir().join(format!("sb-batch-stem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sources = [root.join("one"), root.join("two")];
        let dest = root.join("out");
        for folder in &sources {
            fs::create_dir_all(folder).unwrap();
            fs::write(folder.join("same.3mf"), b"fixture").unwrap();
        }
        fs::create_dir_all(&dest).unwrap();
        let paths = sources.each_ref().map(|dir| dir.join("same.3mf"));
        let planned = plan_batch(&paths, "u1", &[Some("0.4".into()), Some("0.4".into())]).unwrap();
        assert_eq!(planned[0].output, sources[0].join("U1-same.3mf"));
        assert_eq!(planned[1].output, sources[1].join("U1-same.3mf"));
        assert_ne!(planned[0].output, planned[1].output);
        assert!(planned.iter().all(|entry| !entry.output.exists()));
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
    fn removing_one_queued_source_preserves_other_sources_and_finished_receipts() {
        let one = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/bambu-one-color.3mf");
        let two = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/bambu-nozzle-0.6.3mf");
        let one_before = fs::read(&one).unwrap();
        let two_before = fs::read(&two).unwrap();
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.batch_staged = vec![one.clone(), two.clone()];
            state.source = Some(one.clone());
            state.facts = Some(json!({"source":one}));
            state.bundle = Some(json!({"mock":true}));
            state.target = Some("u1".to_owned());
            state.installed_oem = true;
            state.recent_projects = vec![one.clone(), two.clone()];
            state.batch = Some(json!({"phase":"completed","successes":1}));
            state.last_result = Some(json!({"mock":"earlier result"}));
        }
        let outcome = registry.execute("batch.remove", &json!({"path":one}));
        assert_eq!(outcome["ok"], true, "{outcome}");
        assert_eq!(outcome["data"]["removed"], true);
        assert_eq!(outcome["data"]["staged"], 1);
        assert_eq!(outcome["data"]["paths"], json!([two]));
        let state = registry.snapshot();
        assert_eq!(state["batch_selection"]["paths"], json!([two]));
        assert!(state["source"].is_null());
        assert!(state["facts"].is_null());
        assert_eq!(state["installed_oem"], false);
        assert!(state["target"].is_null());
        assert_eq!(state["batch"]["phase"], "completed");
        assert_eq!(state["result"]["mock"], "earlier result");
        assert_eq!(state["projects"], json!([one, two]));
        assert_eq!(fs::read(&one).unwrap(), one_before);
        assert_eq!(fs::read(&two).unwrap(), two_before);

        let outcome = registry.execute("batch.remove", &json!({"path":two}));
        assert_eq!(outcome["ok"], true, "{outcome}");
        assert_eq!(outcome["data"]["staged"], 0);
        assert!(registry.snapshot()["batch_selection"]["paths"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(registry.snapshot()["source"].is_null());
        assert_eq!(fs::read(&one).unwrap(), one_before);
        assert_eq!(fs::read(&two).unwrap(), two_before);
    }

    #[test]
    fn queue_removal_is_denied_if_missing_or_busy_without_mutating_state() {
        let one = PathBuf::from("/read-only/queued.3mf");
        let registry = Registry::new();
        {
            let mut state = registry.state.lock().unwrap();
            state.batch_staged = vec![one.clone()];
            state.source = Some(one.clone());
            state.phase = "running".into();
        }
        let before = registry.snapshot();
        assert_eq!(
            registry.execute("batch.remove", &json!({"path":one}))["ok"],
            false
        );
        assert_eq!(registry.snapshot(), before);
        {
            let mut state = registry.state.lock().unwrap();
            state.phase = "idle".into();
            state.batch = Some(json!({"phase":"running"}));
        }
        let before = registry.snapshot();
        assert_eq!(
            registry.execute("batch.remove", &json!({"path":one}))["ok"],
            false
        );
        assert_eq!(registry.snapshot(), before);
        {
            let mut state = registry.state.lock().unwrap();
            state.batch = None;
        }
        let before = registry.snapshot();
        for input in [json!({}), json!({"path":"/read-only/other.3mf"})] {
            assert_eq!(registry.execute("batch.remove", &input)["ok"], false);
            assert_eq!(registry.snapshot(), before);
        }
        assert_eq!(
            registry.execute("batch.remove", &json!({"path":one}))["ok"],
            true
        );
        assert!(registry.snapshot()["source"].is_null());
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
