use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::env;
use std::process;

// The desktop's loopback endpoint shares its Registry with the visible GPUI app.
// Standalone commands intentionally retain their separate in-process Registry.
const DESKTOP_ATTACH: &str = "127.0.0.1:48637";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let registry = Registry::new();
    let result: Result<Value, String> = match args.as_slice() {
        [mode] if mode == "--mcp" => protocol::stdio(&registry, true)
            .map(|_| json!({"silent":true})).map_err(|e|e.to_string()),
        [mode] if mode == "--stdio" => protocol::stdio(&registry, false)
            .map(|_| json!({"silent":true})).map_err(|e|e.to_string()),
        [mode, addr] if mode == "--serve" => {
            match protocol::start_attach(registry, addr) {
                Ok(bound) => { eprintln!("SliceBridge local attach listening on {bound}");
                    loop { std::thread::park(); }
                }
                Err(e) => Err(e.to_string()),
            }
        }
        // Explicitly address the *visible* desktop session; never fake a stage
        // success by quietly falling back to the CLI's fresh Registry.
        [mode, command] if mode == "--desktop" => protocol::attach(
            DESKTOP_ATTACH,
            command,
            json!({}),
        )
        .map_err(|e| format!("desktop GUI unavailable at {DESKTOP_ATTACH}: {e}")),
        [mode, command, params] if mode == "--desktop" => {
            match serde_json::from_str::<Value>(params) {
                Ok(value) => protocol::attach(DESKTOP_ATTACH, command, value)
                    .map_err(|e| format!("desktop GUI unavailable at {DESKTOP_ATTACH}: {e}")),
                Err(e) => Err(format!("invalid input JSON: {e}")),
            }
        }
        [mode, addr, command] if mode == "--attach" =>
            protocol::attach(addr, command, json!({})).map_err(|e|e.to_string()),
        [mode, addr, command, params] if mode == "--attach" => {
            match serde_json::from_str::<Value>(params) {
                Ok(value) => protocol::attach(addr, command, value).map_err(|e|e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
        [command] => Ok(registry.execute(command, &json!({}))),
        [command, params] => match serde_json::from_str::<Value>(params) {
            Ok(value) => Ok(registry.execute(command, &value)),
            Err(e) => Err(format!("invalid input JSON: {e}")),
        },
        _ => Err("usage: slicebridge-alpha [command JSON] | --desktop command [JSON] | --mcp | --stdio | --serve 127.0.0.1:48637 | --attach 127.0.0.1:48637 command JSON".into()),
    };
    match result {
        Ok(reply) => {
            if reply["silent"] != true {
                println!("{reply}");
            }
            if reply.get("ok") == Some(&Value::Bool(false)) {
                process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("SliceBridge Alpha: {error}");
            process::exit(1);
        }
    }
}
