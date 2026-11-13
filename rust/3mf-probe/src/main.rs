use serde_json::json;
use slicebridge_3mf_probe::{inspect_package, strip_slice_cache};
use std::env;
use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let result: Result<serde_json::Value, String> = match args.as_slice() {
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
        [flag, source, destination] if flag == "--strip-cache" => {
            strip_slice_cache(PathBuf::from(source), PathBuf::from(destination))
                .map(|removed| json!({"ok": true, "removed": removed}))
        }
        _ => Err(
            "usage: slicebridge-3mf-probe <file.3mf> | --strip-cache <source.3mf> <output.3mf>"
                .to_owned(),
        ),
    };

    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            process::exit(1);
        }
    }
}
