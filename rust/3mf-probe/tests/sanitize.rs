use slicebridge_3mf_probe::{inspect_package, strip_slice_cache};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

static TEST_ID: AtomicU64 = AtomicU64::new(0);
const MODEL: &[u8] = br#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices><vertex/><vertex/><vertex/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#;
const OPC_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

struct Fixture {
    dir: PathBuf,
    input: PathBuf,
    output: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn fixture() -> Fixture {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "slicebridge-rust-strip-{}-{now}-{}",
        std::process::id(),
        TEST_ID.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("input.3mf");
    let output = dir.join("output.3mf");
    let mut writer = ZipWriter::new(File::create(&input).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in [
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/3D/3dmodel.model" ContentType="model"/><Override PartName="/Metadata/plate_1.gcode" ContentType="cache"/><Override PartName="/Metadata/plate_1.png" ContentType="thumbnail"/></Types>"#.as_slice(),
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="model"/><Relationship Target="/Metadata/plate_1.gcode" Id="cache"/><Relationship Target="/Metadata/plate_1.png" Id="thumb"/></Relationships>"#.as_slice(),
        ),
        ("3D/3dmodel.model", MODEL),
        ("Metadata/plate_1.gcode", b"G1 X1".as_slice()),
        ("Metadata/plate_1.gcode.md5", b"hash".as_slice()),
        ("Metadata/plate_2.json", b"{}".as_slice()),
        ("Metadata/plate_1.png", b"THUMBNAIL".as_slice()),
        (
            "Metadata/project_settings.config",
            br#"{"printer_model":"Kobra 3"}"#.as_slice(),
        ),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    Fixture { dir, input, output }
}
fn read_part(archive: &mut ZipArchive<File>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    Some(bytes)
}

#[test]
fn removes_only_slicer_cache_and_keeps_all_other_payloads() {
    let f = fixture();
    let original = fs::read(&f.input).unwrap();
    assert_eq!(strip_slice_cache(&f.input, &f.output).unwrap(), 3);
    let mut archive = ZipArchive::new(File::open(&f.output).unwrap()).unwrap();
    assert_eq!(archive.len(), 5);
    for cache in [
        "Metadata/plate_1.gcode",
        "Metadata/plate_1.gcode.md5",
        "Metadata/plate_2.json",
    ] {
        assert!(archive.by_name(cache).is_err(), "cache {cache} survived");
    }
    assert_eq!(
        read_part(&mut archive, "Metadata/plate_1.png").unwrap(),
        b"THUMBNAIL"
    );
    assert_eq!(read_part(&mut archive, "3D/3dmodel.model").unwrap(), MODEL);
    let types = String::from_utf8(read_part(&mut archive, "[Content_Types].xml").unwrap()).unwrap();
    let rels = String::from_utf8(read_part(&mut archive, "_rels/.rels").unwrap()).unwrap();
    assert!(!types.contains("plate_1.gcode"));
    assert!(!rels.contains("plate_1.gcode"));
    assert!(types.contains("plate_1.png"));
    assert!(rels.contains("plate_1.png"));
    assert!(rels.contains(OPC_NS));

    assert_eq!(
        fs::read(&f.input).unwrap(),
        original,
        "source must stay unchanged"
    );
    assert_eq!(inspect_package(&f.output).unwrap().slice_caches, 0);
    assert_eq!(inspect_package(&f.output).unwrap().printer_model, "Kobra 3");
}

#[test]
fn refuses_to_overwrite_existing_destinations() {
    let f = fixture();
    fs::write(&f.output, b"sentinel").unwrap();
    assert!(strip_slice_cache(&f.input, &f.output)
        .unwrap_err()
        .contains("overwrite"));
    assert_eq!(fs::read(&f.output).unwrap(), b"sentinel");
    assert!(strip_slice_cache(&f.input, &f.input).is_err());
}

#[test]
fn cli_writes_explicit_opt_in_destination_and_json_count() {
    let f = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .args(["--strip-cache"])
        .arg(&f.input)
        .arg(&f.output)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["removed"], 3);
    assert!(f.output.exists());
}

#[test]
fn preserved_keys_are_written_to_new_output_without_changing_source() {
    use slicebridge_3mf_probe::strip_slice_cache_preserving;

    let f = fixture();
    let original = fs::read(&f.input).unwrap();
    let keys = vec!["flow_ratio".to_owned(), "cooling".to_owned()];
    assert_eq!(
        strip_slice_cache_preserving(&f.input, &f.output, &keys).unwrap(),
        3
    );

    let mut archive = ZipArchive::new(File::open(&f.output).unwrap()).unwrap();
    let settings = read_part(&mut archive, "Metadata/project_settings.config").unwrap();
    let json: serde_json::Value = serde_json::from_slice(&settings).unwrap();
    let differences = json["different_settings_to_system"].as_array().unwrap();
    assert_eq!(
        differences.len(),
        6,
        "four default slots plus two metadata fields"
    );
    assert_eq!(differences[0], "cooling;flow_ratio");
    assert_eq!(json["printer_model"], "Kobra 3");
    assert_eq!(fs::read(&f.input).unwrap(), original);
    assert_eq!(inspect_package(&f.output).unwrap().slice_caches, 0);
}

#[test]
fn cli_accepts_repeated_preserved_process_keys() {
    let f = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .args([
            "--strip-cache",
            "--preserve",
            "flow_ratio",
            "--preserve",
            "cooling",
        ])
        .arg(&f.input)
        .arg(&f.output)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stdout);
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["removed"], 3);
    let mut archive = ZipArchive::new(File::open(&f.output).unwrap()).unwrap();
    let settings = read_part(&mut archive, "Metadata/project_settings.config").unwrap();
    let json: serde_json::Value = serde_json::from_slice(&settings).unwrap();
    assert_eq!(
        json["different_settings_to_system"][0],
        "cooling;flow_ratio"
    );
}

#[test]
fn incomplete_preserve_command_fails_closed_without_creating_output() {
    let f = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .args(["--strip-cache", "--preserve"])
        .arg(&f.input)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(!f.output.exists());
}
