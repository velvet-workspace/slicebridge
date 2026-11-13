use serde_json::{json, Value};
use slicebridge_3mf_probe::{
    inspect_package, map_native_material_settings, source_slot_plan, strip_slice_cache_preserving,
};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::process;

const MAX_JSON_INPUT_BYTES: u64 = 8 << 20;

fn usage() -> String {
    "usage: slicebridge-3mf-probe <file.3mf> | --strip-cache [--preserve KEY]... <source.3mf> <output.3mf> | --plan-slots <request.json> | --map-materials <request.json>"
        .to_owned()
}

fn read_request(filename: &OsStr) -> Result<Value, String> {
    let path = PathBuf::from(filename);
    let mut file = File::open(&path).map_err(|e| format!("request JSON cannot be opened: {e}"))?;
    let mut contents = Vec::new();
    file.by_ref()
        .take(MAX_JSON_INPUT_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|e| format!("request JSON read: {e}"))?;
    if contents.len() as u64 > MAX_JSON_INPUT_BYTES {
        return Err("request JSON exceeds 8 MB limit".to_owned());
    }
    serde_json::from_slice(&contents).map_err(|e| format!("request JSON invalid: {e}"))
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

fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match run(&args) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            process::exit(1);
        }
    }
}
