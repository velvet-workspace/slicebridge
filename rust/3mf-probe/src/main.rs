use serde_json::{json, Value};
use slicebridge_3mf_probe::{
    build_native_conversion_plan, compose_project_settings, convert_archive_experimental,
    convert_native_archive, inspect_package, map_native_material_settings, remap_paint_hex,
    rewrite_model_paint_xml, rewrite_model_settings, rewrite_slice_info_xml, source_slot_plan,
    strip_slice_cache_preserving, validate_native_bundle, SliceInfoSlot,
};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process;

const MAX_JSON_INPUT_BYTES: u64 = 8 << 20;

fn usage() -> String {
    "usage: slicebridge-3mf-probe <file.3mf> | --strip-cache [--preserve KEY]... <source.3mf> <output.3mf> | --plan-slots <request.json> | --map-materials <request.json> | --validate-native-bundle <request.json> | --plan-native-conversion <request.json> | --remap-paint <request.json> | --remap-model <request.json> | --rewrite-model-settings <request.json> | --compose-project-settings <request.json> | --rewrite-slice-info <request.json> | --convert-archive <plan.json> <source.3mf> <output.3mf> | --convert-native-archive <verified-request.json> <source.3mf> <new-output.3mf> | --convert-archive-stdio | --convert-native-archive-stdio"
        .to_owned()
}

fn read_request(filename: &OsStr) -> Result<Value, String> {
    let path = PathBuf::from(filename);
    let mut file = File::open(&path).map_err(|e| format!("request JSON cannot be opened: {e}"))?;
    let mut contents = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_JSON_INPUT_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|e| format!("request JSON read: {e}"))?;
    if contents.len() as u64 > MAX_JSON_INPUT_BYTES {
        return Err("request JSON exceeds 8 MB limit".to_owned());
    }
    serde_json::from_slice(&contents).map_err(|e| format!("request JSON invalid: {e}"))
}

fn request_mapping(request: &Value) -> Result<BTreeMap<u32, u32>, String> {
    let mut mapping = BTreeMap::new();
    let entries = request
        .get("mapping")
        .and_then(Value::as_object)
        .ok_or_else(|| "mapping must be an object of source IDs to target slot IDs".to_owned())?;
    if entries.len() > 64 {
        return Err("mapping contains more than 64 source entries".into());
    }
    for (source, target) in entries {
        let source_id: u32 = source
            .parse()
            .map_err(|_| format!("invalid source filament ID {source:?}"))?;
        let target_id = target
            .as_u64()
            .ok_or_else(|| format!("invalid target filament ID for source {source}"))?;
        if source_id == 0 || !(1..=64).contains(&target_id) {
            return Err("source or target filament ID is outside supported range".into());
        }
        mapping.insert(source_id, target_id as u32);
    }
    Ok(mapping)
}

// The receipt deliberately omits the OEM profile: machine scripts never reach
// stdout. The full Go-compatible plan is only available to in-process callers.
fn plan_receipt(plan: &Value) -> Value {
    json!({
        "profile_name": plan["profile"]["name"],
        "printer_model_id": plan["profile"]["printer_model_id"],
        "slot_count": plan["slot_count"],
        "slots": plan["slots"],
        "mapping": plan["mapping"],
        "supports": plan["supports"],
        "preserved": plan["preserved"],
    })
}

fn run(args: &[OsString]) -> Result<Value, String> {
    match args {
        [source] => inspect_package(PathBuf::from(source)).map(|facts| {
            json!({
                "ok": true,
                "facts": {
                    "vertices": facts.vertices,
                    "triangles": facts.triangles,
                    "slice_caches": facts.slice_caches,
                    "printer_model": facts.printer_model,
                }
            })
        }),
        [flag, plan_file, source, destination] if flag == OsStr::new("--convert-archive") => {
            let plan = read_request(plan_file)?;
            let removed = convert_archive_experimental(
                PathBuf::from(source),
                PathBuf::from(destination),
                &plan,
            )?;
            Ok(json!({"ok": true, "removed": removed, "experimental": true}))
        }
        [flag, request, source, destination] if flag == OsStr::new("--convert-native-archive") => {
            let value = read_request(request)?;
            convert_native_archive(&value, &PathBuf::from(source), &PathBuf::from(destination))
        }
        [flag, source] if flag == OsStr::new("--plan-slots") => {
            let value = read_request(source)?;
            let result = source_slot_plan(&value)?;
            Ok(json!({"ok": true, "plan": result}))
        }
        [flag, source] if flag == OsStr::new("--map-materials") => {
            let value = read_request(source)?;
            let result = map_native_material_settings(&value)?;
            Ok(json!({"ok": true, "project_settings": result["project_settings"]}))
        }
        [flag, source] if flag == OsStr::new("--validate-native-bundle") => {
            let value = read_request(source)?;
            let validation = validate_native_bundle(&value)?;
            Ok(json!({"ok": true, "experimental": true, "validation": validation}))
        }
        [flag, source] if flag == OsStr::new("--plan-native-conversion") => {
            let value = read_request(source)?;
            let plan = build_native_conversion_plan(&value)?;
            Ok(json!({"ok": true, "experimental": true, "receipt": plan_receipt(&plan)}))
        }
        [flag, source] if flag == OsStr::new("--compose-project-settings") => {
            let input = read_request(source)?;
            let settings = compose_project_settings(&input)?;
            Ok(json!({"ok": true, "project_settings": settings}))
        }
        [flag, source] if flag == OsStr::new("--rewrite-slice-info") => {
            let input = read_request(source)?;
            let mut mapping = BTreeMap::new();
            for (original, target) in request_mapping(&input)? {
                let original = i32::try_from(original).map_err(|_| {
                    "slice_info source filament ID exceeds signed integer limit".to_owned()
                })?;
                mapping.insert(original, target as i32);
            }
            let xml = input
                .get("slice_info_xml")
                .and_then(Value::as_str)
                .ok_or_else(|| "slice_info_xml must be a UTF-8 XML string".to_owned())?;
            let model_id = input
                .get("model_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "model_id must be a string".to_owned())?;
            let entries = input
                .get("slots")
                .and_then(Value::as_array)
                .ok_or_else(|| "slots must be an array".to_owned())?;
            if entries.is_empty() || entries.len() > 64 {
                return Err("slots must contain 1..64 target entries".into());
            }
            let mut slots = Vec::with_capacity(entries.len());
            for (index, item) in entries.iter().enumerate() {
                let color = item
                    .get("color")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("slot {index} color must be a string"))?;
                let material = item
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("slot {index} type must be a string"))?;
                slots.push(SliceInfoSlot {
                    color: color.to_owned(),
                    material: material.to_owned(),
                });
            }
            let output = rewrite_slice_info_xml(xml.as_bytes(), &slots, &mapping, model_id)?;
            Ok(
                json!({"ok": true, "slice_info_xml": String::from_utf8(output)
                .map_err(|e| format!("slice info output not UTF-8: {e}"))?}),
            )
        }
        [flag, source] if flag == OsStr::new("--remap-paint") => {
            let value = read_request(source)?;
            let mapping = request_mapping(&value)?;
            if let Some(samples) = value.get("samples").and_then(Value::as_array) {
                if samples.len() > 10_000 {
                    return Err("too many paint samples".to_owned());
                }
                let results = samples
                    .iter()
                    .map(|sample| {
                        let hex = sample
                            .as_str()
                            .ok_or_else(|| "paint sample must be a string".to_owned())?;
                        remap_paint_hex(hex, &mapping)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"ok": true, "samples": results}))
            } else {
                let hex = value
                    .get("paint")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "paint must be a hex string".to_owned())?;
                Ok(json!({"ok": true, "paint": remap_paint_hex(hex, &mapping)?}))
            }
        }
        [flag, source] if flag == OsStr::new("--remap-model") => {
            let value = read_request(source)?;
            let mapping = request_mapping(&value)?;
            let xml = value
                .get("model_xml")
                .and_then(Value::as_str)
                .ok_or_else(|| "model_xml must be a UTF-8 XML string".to_owned())?;
            let output = rewrite_model_paint_xml(xml.as_bytes(), &mapping)?;
            Ok(json!({"ok": true, "model_xml": String::from_utf8(output)
                .map_err(|e| format!("model output not UTF-8: {e}"))?}))
        }
        [flag, source] if flag == OsStr::new("--rewrite-model-settings") => {
            let value = read_request(source)?;
            let map = request_mapping(&value)?;
            let mut mapping = BTreeMap::new();
            for (source_id, target) in map {
                let source_id = i32::try_from(source_id).map_err(|_| {
                    "model settings source filament ID exceeds signed integer limit".to_owned()
                })?;
                mapping.insert(source_id, target as i32);
            }
            let xml = value
                .get("model_settings_xml")
                .and_then(Value::as_str)
                .ok_or_else(|| "model_settings_xml must be a UTF-8 XML string".to_owned())?;
            let slots = value
                .get("slots")
                .and_then(Value::as_u64)
                .and_then(|count| usize::try_from(count).ok())
                .ok_or_else(|| "slots must be a positive integer".to_owned())?;
            let output = rewrite_model_settings(xml.as_bytes(), &mapping, slots)?;
            Ok(
                json!({"ok": true, "model_settings_xml": String::from_utf8(output)
                .map_err(|e| format!("model settings output not UTF-8: {e}"))?}),
            )
        }
        [flag, rest @ ..] if flag == OsStr::new("--strip-cache") => {
            let mut at = 0usize;
            let mut preserved = Vec::new();
            while at < rest.len() && rest[at] == OsStr::new("--preserve") {
                let key = rest
                    .get(at + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(usage)?;
                preserved.push(key.to_owned());
                at += 2;
            }
            match &rest[at..] {
                [source, destination] => strip_slice_cache_preserving(
                    PathBuf::from(source),
                    PathBuf::from(destination),
                    &preserved,
                )
                .map(|removed| json!({"ok": true, "removed": removed})),
                _ => Err(usage()),
            }
        }
        _ => Err(usage()),
    }
}

// Opt-in, finite JSONL transport for local *experimental* conversion QA.
// One process can process multiple independent requests without repeated
// operating-system startup cost. No network listener, daemon, printer command
// or installed-profile lookup is created. It exits cleanly on stdin EOF.
fn convert_archive_stdio(request: &Value) -> Result<Value, String> {
    let plan = read_request(OsStr::new(job_path(request, "plan_file")?))?;
    let source = PathBuf::from(job_path(request, "source")?);
    let destination = PathBuf::from(job_path(request, "destination")?);
    let removed = convert_archive_experimental(source, destination, &plan)?;
    Ok(json!({"ok":true,"removed":removed,"experimental":true}))
}

// Native jobs reuse the JSONL transport above. Every destination named by a
// job that passes field validation is claimed for the life of this worker, so
// a repeated output is refused before any filesystem work, even if the first
// attempt failed and left nothing behind. Outputs are never overwritten.
fn convert_native_archive_stdio(
    claimed: &mut BTreeSet<String>,
    job: &Value,
) -> Result<Value, String> {
    let request_file = job_path(job, "request_file")?;
    let source = PathBuf::from(job_path(job, "source")?);
    let destination = job_path(job, "destination")?;
    if !claimed.insert(destination.to_owned()) {
        return Err("duplicate destination in this worker; refusing overwrite".into());
    }
    let verified = read_request(OsStr::new(request_file))?;
    convert_native_archive(&verified, &source, &PathBuf::from(destination))
}

fn job_path<'a>(request: &'a Value, name: &str) -> Result<&'a str, String> {
    request
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or_else(|| format!("{name} must be a bounded nonempty path"))
}

fn run_stdio() -> Result<(), String> {
    run_jsonl_worker(convert_archive_stdio)
}

fn run_native_stdio() -> Result<(), String> {
    let mut claimed = BTreeSet::new();
    run_jsonl_worker(|job| convert_native_archive_stdio(&mut claimed, job))
}

// Shared bounded transport: one JSON object per line, at most 8 MB per line and
// 4096 jobs per process. Each job produces exactly one reply line; a job error
// is reported and the worker continues. Framing errors end the worker.
fn run_jsonl_worker(mut handle: impl FnMut(&Value) -> Result<Value, String>) -> Result<(), String> {
    const MAX_REQUESTS: usize = 4096;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    for _ in 0..MAX_REQUESTS {
        let mut line = Vec::new();
        let count = (&mut reader)
            .take(MAX_JSON_INPUT_BYTES + 2)
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("stdio request read: {e}"))?;
        if count == 0 {
            return Ok(());
        }
        if line.len() as u64 > MAX_JSON_INPUT_BYTES {
            writer
                .write_all(b"{\"ok\":false,\"error\":\"stdio request exceeds 8 MB limit\"}\n")
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            return Err("oversized stdio request; refusing further unframed input".into());
        }
        let result = serde_json::from_slice::<Value>(&line)
            .map_err(|e| format!("stdio request invalid JSON: {e}"))
            .and_then(|request| handle(&request));
        let reply = match result {
            Ok(response) => response,
            Err(error) => json!({"ok":false,"error":error}),
        };
        writer
            .write_all(reply.to_string().as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|e| format!("stdio response write: {e}"))?;
    }
    Err("stdio worker reached maximum 4096 requests; restart required".into())
}

fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.as_slice() == [OsStr::new("--convert-archive-stdio")] {
        if let Err(error) = run_stdio() {
            eprintln!("experimental stdio worker terminated: {error}");
            process::exit(1);
        }
        return;
    }
    if args.as_slice() == [OsStr::new("--convert-native-archive-stdio")] {
        if let Err(error) = run_native_stdio() {
            eprintln!("experimental native stdio worker terminated: {error}");
            process::exit(1);
        }
        return;
    }
    match run(&args) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            process::exit(1);
        }
    }
}
