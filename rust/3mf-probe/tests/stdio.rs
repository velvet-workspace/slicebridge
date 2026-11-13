use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn experimental_stdio_worker_replies_per_line_and_exits_on_eof() {
    let bin = env!("CARGO_BIN_EXE_slicebridge-3mf-probe");
    let mut child = Command::new(bin)
        .arg("--convert-archive-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch isolated Rust 3MF stdio worker");

    {
        let mut stdin = child.stdin.take().expect("worker stdin");
        stdin
            .write_all(b"{not-json}\n{}\n{\"source\":\"a.3mf\"}\n")
            .expect("send malformed and incomplete jobs");
    }
    let output = child.wait_with_output().expect("wait for EOF exit");
    assert!(
        output.status.success(),
        "stdio worker did not exit on EOF: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses: Vec<Value> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("one JSON reply per request"))
        .collect();
    assert_eq!(responses.len(), 3);
    for response in responses {
        assert_eq!(response["ok"], false);
        assert!(response["error"].as_str().is_some_and(|s| !s.is_empty()));
    }
}
