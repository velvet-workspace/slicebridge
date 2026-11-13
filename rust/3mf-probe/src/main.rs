use serde_json::json;
use slicebridge_3mf_probe::inspect_package;
use std::env;
use std::path::PathBuf;
use std::process;

fn main() {
    let mut args = env::args_os().skip(1);
    let path = args.next().map(PathBuf::from);
    if path.is_none() || args.next().is_some() {
        println!(
            "{}",
            json!({"ok": false, "error": "usage: slicebridge-3mf-probe <file.3mf>"})
        );
        process::exit(2);
    }
    match inspect_package(path.expect("checked above")) {
        Ok(facts) => println!(
            "{}",
            json!({
                "ok": true,
                "facts": {
                    "vertices": facts.vertices,
                    "triangles": facts.triangles,
                    "slice_caches": facts.slice_caches,
                    "printer_model": facts.printer_model,
                }
            })
        ),
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            process::exit(1);
        }
    }
}
