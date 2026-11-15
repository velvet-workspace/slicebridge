use serde_json::Value;
use std::process::Command;

#[test]
fn packaged_native_worker_exposes_bounded_read_only_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg("--identify-native-engine")
        .output()
        .expect("run the exact built Rust converter");
    assert!(output.status.success(), "identity command failed");
    assert!(output.stdout.len() <= 4096, "identity output is oversized");
    assert!(output.stderr.is_empty(), "identity must not emit errors");
    let reply: Value = serde_json::from_slice(&output.stdout).expect("valid JSON identity");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["engine"], "slicebridge-3mf-probe");
    assert_eq!(reply["native_archive"], true);
    assert_eq!(reply["source_owned"], true);
}
