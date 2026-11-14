use slicebridge_3mf_probe::inspect_package;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const MODEL: &[u8] = br#"<?xml version="1.0"?>
<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
<resources><object id="1" type="model"><mesh>
<vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
<triangles><triangle v1="0" v2="1" v3="2"/></triangles>
</mesh></object></resources></model>"#;

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Test3mf(PathBuf);
impl Drop for Test3mf {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
impl AsRef<Path> for Test3mf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

fn make_archive(entries: &[(&str, &[u8])]) -> Test3mf {
    let path = std::env::temp_dir().join(format!(
        "slicebridge-rust-probe-{}-{}.3mf",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    Test3mf(path)
}

fn valid_entries() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/3dmodel.model", MODEL),
        ("Metadata/plate_1.gcode", b"G1 X1"),
        ("Metadata/plate_1.gcode.md5", b"abc"),
        ("Metadata/plate_1.png", b"thumbnail"),
        (
            "Metadata/project_settings.config",
            br#"{"printer_model":"U1","printer_settings_id":"fallback"}"#,
        ),
    ]
}

#[test]
fn real_editable_model_facts_match_go_contract() {
    let file = make_archive(&valid_entries());
    let before = fs::read(file.as_ref()).unwrap();
    let facts = inspect_package(file.as_ref()).unwrap();
    assert_eq!(facts.vertices, 3);
    assert_eq!(facts.triangles, 1);
    assert_eq!(facts.slice_caches, 2);
    assert_eq!(facts.printer_model, "U1");
    assert_eq!(
        before,
        fs::read(file.as_ref()).unwrap(),
        "read-only inspection must not mutate"
    );
}

#[test]
fn printer_settings_id_is_fallback() {
    let file = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/a.model", MODEL),
        (
            "Metadata/project_settings.config",
            br#"{"printer_model":"","printer_settings_id":"Creator 5"}"#,
        ),
    ]);
    assert_eq!(inspect_package(file).unwrap().printer_model, "Creator 5");
}

#[test]
fn missing_types_or_geometry_fails_closed() {
    let no_types = make_archive(&[("3D/a.model", MODEL)]);
    assert!(inspect_package(no_types)
        .unwrap_err()
        .contains("Content_Types"));
    let sliced_only = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("Metadata/plate_1.gcode", b"G1 X1"),
    ]);
    assert!(inspect_package(sliced_only)
        .unwrap_err()
        .contains("editable 3D mesh"));
}

#[test]
fn malformed_3d_xml_and_json_are_rejected() {
    let invalid_xml = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/a.model", b"<model><vertex></triangle>"),
    ]);
    assert!(inspect_package(invalid_xml).is_err());
    let invalid_json = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/a.model", MODEL),
        ("Metadata/project_settings.config", b"{unquoted}"),
    ]);
    assert!(inspect_package(invalid_json).unwrap_err().contains("JSON"));
}

#[test]
fn traversal_and_duplicate_names_are_rejected() {
    let traversal = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/a.model", MODEL),
        ("../bad.txt", b"evil"),
    ]);
    assert!(inspect_package(traversal)
        .unwrap_err()
        .contains("unsafe archive"));
    let duplicate = make_archive(&[
        ("[Content_Types].xml", b"<Types/>"),
        ("3D/a.model", MODEL),
        ("3D/b.model", MODEL),
    ]);
    // zip::ZipWriter forbids duplicate names; construct the invalid ZIP by
    // patching both the local and central directory name bytes (same length).
    let mut data = fs::read(duplicate.as_ref()).unwrap();
    let old = b"3D/b.model";
    let new = b"3D/a.model";
    let mut changed = 0;
    for index in 0..=data.len() - old.len() {
        if &data[index..index + old.len()] == old {
            data[index..index + new.len()].copy_from_slice(new);
            changed += 1;
        }
    }
    assert_eq!(changed, 2);
    fs::write(duplicate.as_ref(), data).unwrap();
    assert!(inspect_package(duplicate)
        .unwrap_err()
        .contains("duplicate archive entry"));
}

#[test]
fn cli_emits_machine_readable_json() {
    let file = make_archive(&valid_entries());
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg(file.as_ref())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["facts"]["vertices"], 3);
    assert_eq!(reply["facts"]["printer_model"], "U1");
}

#[test]
fn oversized_model_member_rejected_from_zip_metadata_before_xml_allocation() {
    // 201 MiB *uncompressed*, but highly deflatable. Memory stays bounded:
    // generate in 1 MiB chunks without constructing a 201 MiB Vec.
    let path = std::env::temp_dir().join(format!(
        "slicebridge-model-overflow-{}-{}.3mf",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let file = Test3mf(path);
    let mut writer = ZipWriter::new(File::create(file.as_ref()).unwrap());
    // Fast low-compression deflate avoids a costly full CPU compression
    // benchmark in a security-limit regression while keeping scratch small.
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    writer.start_file("[Content_Types].xml", opts).unwrap();
    writer.write_all(b"<Types/>").unwrap();
    writer.start_file("3D/3dmodel.model", opts).unwrap();
    let chunk = [b'x'; 1 << 20];
    for _ in 0..201 {
        writer.write_all(&chunk).unwrap();
    }
    writer.finish().unwrap();
    let compressed = fs::metadata(file.as_ref()).unwrap().len();
    assert!(
        compressed < 2 << 20,
        "test fixture unexpectedly uses too much disk space: {compressed}"
    );
    let error = inspect_package(file.as_ref()).unwrap_err();
    assert!(
        error.contains("model XML exceeds 200 MiB"),
        "oversize ZIP model was not rejected before XML parsing: {error}"
    );
    let reply = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg(file.as_ref())
        .output()
        .unwrap();
    assert!(!reply.status.success());
    let answer: serde_json::Value = serde_json::from_slice(&reply.stdout).unwrap();
    assert_eq!(answer["ok"], false);
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or("")
            .contains("model XML exceeds 200 MiB"),
        "CLI must preserve the precise oversized-model rejection: {answer}"
    );
}

#[test]
fn corrupted_archive_is_rejected() {
    let file = make_archive(&valid_entries());
    let mut data = Vec::new();
    File::open(file.as_ref())
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    fs::write(file.as_ref(), &data[..data.len() / 2]).unwrap();
    assert!(inspect_package(file).is_err());
}
