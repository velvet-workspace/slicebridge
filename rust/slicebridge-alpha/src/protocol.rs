//! Plain JSON-lines CLI, MCP stdio, and loopback attach share one registry.
use crate::{Registry, COMMANDS};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;

fn schema(name: &str) -> Value {
    let required = match name {
        "source.inspect" | "source.select" | "bundle.load" => vec!["path"],
        "printer.select" => vec!["name"],
        "profiles.select" => vec!["target"],
        "profiles.materials" => vec!["material"],
        "material.select" => vec!["material", "preset"],
        "conversion.start" => vec!["output"],
        "slicer.open" => vec!["executable"],
        _ => vec![],
    };
    json!({"type":"object","properties":{
        "path":{"type":"string"},
        "output":{"type":"string"},
        "engine":{"type":"string"},
        "executable":{"type":"string"},
        "name":{"type":"string"},
        "target":{"type":"string","enum":["u1","qidi"]},
        "nozzle":{"type":"string","enum":["0.2","0.4","0.6","0.8"]},
        "material":{"type":"string","enum":["PLA","PETG","ABS","ASA","TPU"]},
        "preset":{"type":"string"},
        "slot":{"type":"integer","minimum":1,"maximum":64}
    },"required":required,"additionalProperties":false})
}

pub fn mcp_reply(registry: &Registry, request: &Value) -> Option<Value> {
    let method = request.get("method")?.as_str()?;
    let id = request.get("id")?.clone();
    let result = match method {
        "initialize" => json!({"protocolVersion":"2025-06-18",
            "capabilities":{"tools":{"listChanged":false}},
            "serverInfo":{"name":"slicebridge-alpha","version":env!("CARGO_PKG_VERSION")}}),
        "ping" => json!({}),
        "tools/list" => json!({"tools":COMMANDS.iter().map(|(name, description)|
            json!({"name":name,"description":description,"inputSchema":schema(name)}))
            .collect::<Vec<_>>()}),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let output = registry.execute(name, &args);
            let error = output.get("ok") == Some(&Value::Bool(false));
            json!({"content":[{"type":"text","text":output.to_string()}],
                "structuredContent":output, "isError":error})
        }
        _ => {
            return Some(json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":"method not found"}}))
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

pub fn command_reply(registry: &Registry, request: &Value) -> Value {
    let name = request.get("command").and_then(Value::as_str).unwrap_or("");
    let args = request.get("args").cloned().unwrap_or_else(|| json!({}));
    registry.execute(name, &args)
}

fn process_lines<R: BufRead, W: Write>(
    registry: &Registry,
    mut reader: R,
    mut output: W,
    mcp: bool,
) -> io::Result<()> {
    let mut raw = String::new();
    loop {
        raw.clear();
        if reader.read_line(&mut raw)? == 0 {
            break;
        }
        let response = match serde_json::from_str::<Value>(raw.trim()) {
            Ok(value) => {
                if mcp {
                    mcp_reply(registry, &value)
                } else {
                    Some(command_reply(registry, &value))
                }
            }
            Err(error) => Some(json!({"jsonrpc":"2.0","id":null,
                "error":{"code":-32700,"message":format!("invalid JSON: {error}")}})),
        };
        if let Some(reply) = response {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn stdio(registry: &Registry, mcp: bool) -> io::Result<()> {
    process_lines(registry, io::stdin().lock(), io::stdout().lock(), mcp)
}

fn handle_stream(registry: Registry, stream: TcpStream) -> io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    process_lines(&registry, reader, stream, false)
}

/// Local-only endpoint for agents attaching to a running desktop session.
pub fn start_attach(registry: Registry, addr: &str) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    if !bound.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attach must use loopback",
        ));
    }
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let session = registry.clone();
            thread::spawn(move || {
                let _ = handle_stream(session, stream);
            });
        }
    });
    Ok(bound)
}

pub fn attach(addr: &str, command: &str, args: Value) -> io::Result<Value> {
    let target: SocketAddr = addr.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "attach address must be IP:port",
        )
    })?;
    if !target.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attach only permits loopback",
        ));
    }
    let mut stream = TcpStream::connect_timeout(&target, std::time::Duration::from_secs(3))?;
    writeln!(stream, "{}", json!({"command":command,"args":args}))?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    serde_json::from_str(&response).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_tools_share_exact_registry() {
        let response = mcp_reply(
            &Registry::new(),
            &json!({"jsonrpc":"2.0","id":5,
            "method":"tools/list"}),
        )
        .unwrap();
        let names = response["result"]["tools"].as_array().unwrap();
        assert_eq!(names.len(), COMMANDS.len());
        assert_eq!(response["id"], 5);
    }

    #[test]
    fn mcp_call_reports_structured_error() {
        let reply = mcp_reply(
            &Registry::new(),
            &json!({"jsonrpc":"2.0","id":6,
            "method":"tools/call","params":{"name":"printer.print","arguments":{}}}),
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }
}
