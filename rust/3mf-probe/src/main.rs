use serde_json::{json, Value};
use slicebridge_3mf_probe::{inspect_package, strip_slice_cache_preserving};
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process;

fn usage() -> String {
    "usage: slicebridge-3mf-probe <file.3mf> | --strip-cache [--preserve KEY]... <source.3mf> <output.3mf>"
        .to_owned()
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
