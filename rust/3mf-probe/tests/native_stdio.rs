use serde_json::{json, Value};
use slicebridge_3mf_probe::{convert_native_archive, inspect_package};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

// Synthetic fixture mirrored from tests/native_archive.rs. Every OEM value
// here is invented for QA; no installed or vendor profile is read.
const SECRET_START: &str = "OEM_START_SECRET_GCODE";
const SOURCE_SETTINGS: &str =
    r#"{"printer_model":"Source Printer X","enable_support":"0","layer_height":"0.2"}"#;
const MODEL: &[u8] = br#"<?xml version="1.0"?>
<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
<resources><object id="1" type="model"><mesh>
<vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
<triangles><triangle v1="0" v2="1" v3="2"/></triangles>
</mesh></object></resources></model>"#;
const SLICE_INFO: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <plate>
    <metadata key="index" value="1"/>
    <filament id="1" type="PLA" color="#FF0000" used_m="1.00" used_g="3.00"/>
    <filament id="2" type="PLA" color="#00FF00" used_m="1.00" used_g="3.00"/>
    <filament id="3" type="PLA" color="#0000FF" used_m="1.00" used_g="3.00"/>
    <filament id="4" type="PLA" color="#FFFF00" used_m="1.00" used_g="3.00"/>
    <filament id="7" type="PETG" color="#123456" used_m="1.00" used_g="3.00"/>
  </plate>
</config>"##;
const MODEL_SETTINGS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="1">
    <metadata key="name" value="Synthetic part"/>
    <metadata key="extruder" value="7"/>
  </object>
</config>"#;

// Mirrors the 8 MB framing limit and 4096-job ceiling in src/main.rs.
const LINE_LIMIT: usize = 8 << 20;
const MAX_JOBS: usize = 4096;
const RECEIPT: &str = r#"{"ok":true,"experimental":true,"removed":2,"planner":"rust-native"}"#;

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Owns a private scratch directory with a synthetic source and request files.
/// Dropping it removes everything the test created.
struct Fixture {
    dir: PathBuf,
    source: PathBuf,
    request: PathBuf,
    bad_request: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "slicebridge-native-stdio-{}-{name}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.3mf");
        write_source(&source);
        let request = dir.join("request.json");
        fs::write(&request, request_value().to_string()).unwrap();
        // Same bundle, but one filament color no longer matches the source.
        let mut wrong = request_value();
        wrong["filaments"][0]["color"] = json!("#FF0001");
        let bad_request = dir.join("bad-request.json");
        fs::write(&bad_request, wrong.to_string()).unwrap();
        Fixture {
            dir,
            source,
            request,
            bad_request,
        }
    }

    fn output(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Fails if the scratch directory holds anything beyond the expected names,
    /// including partial snapshot files left behind by a conversion.
    fn assert_no_residue(&self, expected: &[&str]) {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        want.sort();
        assert_eq!(names, want, "unexpected files in scratch directory");
    }
}

fn write_source(path: &Path) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in [
        ("[Content_Types].xml", b"<Types/>".as_slice()),
        ("3D/3dmodel.model", MODEL),
        (
            "Metadata/project_settings.config",
            SOURCE_SETTINGS.as_bytes(),
        ),
        ("Metadata/slice_info.config", SLICE_INFO),
        ("Metadata/model_settings.config", MODEL_SETTINGS),
        ("Metadata/plate_1.gcode", b"G1 X1".as_slice()),
        ("Metadata/plate_1.gcode.md5", b"hash".as_slice()),
        ("Metadata/plate_1.png", b"THUMBNAIL".as_slice()),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

/// Synthetic installed U1 bundle and five filaments mirroring the source palette.
fn request_value() -> Value {
    json!({
        "bundle": {
            "target": "u1",
            "nozzle": "0.4",
            "profile": {
                "name": "u1",
                "display_name": "Snapmaker U1",
                "printer_model_id": "Snapmaker U1",
                "filament_slots": 4,
                "filament_profiles": {"PLA": "Synthetic PLA", "PETG": "Synthetic PETG"},
                "project_settings": {
                    "printer_model": "Snapmaker U1",
                    "nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"],
                    "machine_start_gcode": SECRET_START,
                    "machine_end_gcode": "END",
                    "layer_height": "0.2",
                    "min_layer_height": ["0.08"],
                    "max_layer_height": ["0.3"],
                    "filament_colour": ["#000000"]
                }
            },
            "material_settings": {
                "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"], "filament_settings_id": ["Synthetic PLA"]},
                "PETG": {"nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"], "filament_settings_id": ["Synthetic PETG"]}
            }
        },
        "filaments": [
            {"id": 1, "color": "#FF0000", "type": "PLA"},
            {"id": 2, "color": "#00FF00", "type": "PLA"},
            {"id": 3, "color": "#0000FF", "type": "PLA"},
            {"id": 4, "color": "#FFFF00", "type": "PLA"},
            {"id": 7, "color": "#123456", "type": "PETG"}
        ],
        "source_settings": serde_json::from_str::<Value>(SOURCE_SETTINGS).unwrap()
    })
}

/// Reads every member into memory so tests can compare semantic content.
fn members(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.insert(entry.name().to_owned(), bytes);
    }
    out
}

fn job(request_file: &Path, source: &Path, destination: &Path) -> String {
    json!({
        "request_file": request_file,
        "source": source,
        "destination": destination,
    })
    .to_string()
        + "\n"
}

/// Starts one worker process, feeds it `input`, and waits for a clean exit.
/// Stdin is written from a helper thread so a full stdout pipe cannot deadlock
/// the test while the worker is still replying.
fn run_worker(input: Vec<u8>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg("--convert-native-archive-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch native stdio worker");
    let mut stdin = child.stdin.take().expect("worker stdin");
    let writer = std::thread::spawn(move || {
        // The worker may legitimately stop reading early (framing or job cap);
        // a broken pipe is then expected, not a test failure.
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().expect("wait for worker exit");
    writer.join().unwrap();
    output
}

fn replies(output: &Output) -> Vec<Value> {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        !stdout.contains(SECRET_START),
        "OEM script leaked on stdout"
    );
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON reply per job"))
        .collect()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn sequential_jobs_convert_and_invalid_or_duplicate_jobs_do_not_stop_worker() {
    let fixture = Fixture::new("sequence");
    let first = fixture.output("first.3mf");
    let second = fixture.output("second.3mf");
    let reference = fixture.output("reference.3mf");
    convert_native_archive(&request_value(), &fixture.source, &reference).unwrap();

    let input = [
        job(&fixture.request, &fixture.source, &first),
        // Missing source field: malformed job between two valid ones.
        json!({"request_file": fixture.request, "destination": fixture.output("x.3mf")})
            .to_string()
            + "\n",
        job(&fixture.request, &fixture.source, &second),
        // Duplicate output with a bad request: must not touch the first file.
        job(&fixture.bad_request, &fixture.source, &first),
        // Same destination with a valid request is still a duplicate.
        job(&fixture.request, &fixture.source, &first),
        // Invalid palette for a fresh destination: nothing may be created.
        job(
            &fixture.bad_request,
            &fixture.source,
            &fixture.output("bad.3mf"),
        ),
    ]
    .concat();
    let source_before = fs::read(&fixture.source).unwrap();
    let request_before = fs::read(&fixture.request).unwrap();

    let output = run_worker(input.into_bytes());
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
    let replies = replies(&output);
    assert_eq!(replies.len(), 6);
    assert_eq!(replies[0], serde_json::from_str::<Value>(RECEIPT).unwrap());
    assert_eq!(replies[1]["ok"], false);
    assert!(replies[1]["error"].as_str().unwrap().contains("source"));
    assert_eq!(replies[2], serde_json::from_str::<Value>(RECEIPT).unwrap());
    assert_eq!(replies[3]["ok"], false);
    assert!(replies[3]["error"].as_str().unwrap().contains("duplicate"));
    assert_eq!(replies[4]["ok"], false);
    assert!(replies[4]["error"].as_str().unwrap().contains("duplicate"));
    assert_eq!(replies[5]["ok"], false);
    assert!(replies[5]["error"]
        .as_str()
        .unwrap()
        .contains("does not match source slice_info"));

    // The first output is still the valid conversion, not the bad-request or
    // second-job overwrite.
    assert_eq!(members(&first), members(&reference));
    assert_eq!(
        inspect_package(&second).unwrap().printer_model,
        "Snapmaker U1"
    );
    assert_eq!(fs::read(&fixture.source).unwrap(), source_before);
    assert_eq!(fs::read(&fixture.request).unwrap(), request_before);
    fixture.assert_no_residue(&[
        "source.3mf",
        "request.json",
        "bad-request.json",
        "first.3mf",
        "second.3mf",
        "reference.3mf",
    ]);
}

#[test]
fn restarted_worker_refuses_existing_output_unchanged_and_converts_new_one() {
    let fixture = Fixture::new("restart");
    let first = fixture.output("first.3mf");
    let input = job(&fixture.request, &fixture.source, &first);
    let started = run_worker(input.into_bytes());
    assert!(started.status.success(), "{}", stderr(&started));
    let first_bytes = fs::read(&first).unwrap();

    let second = fixture.output("second.3mf");
    let restarted = run_worker(
        [
            job(&fixture.request, &fixture.source, &first),
            job(&fixture.request, &fixture.source, &second),
        ]
        .concat()
        .into_bytes(),
    );
    assert!(restarted.status.success(), "{}", stderr(&restarted));
    let replies = replies(&restarted);
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["ok"], false);
    assert!(replies[0]["error"]
        .as_str()
        .unwrap()
        .contains("already exists"));
    assert_eq!(replies[1], serde_json::from_str::<Value>(RECEIPT).unwrap());
    assert_eq!(fs::read(&first).unwrap(), first_bytes);
    fixture.assert_no_residue(&[
        "source.3mf",
        "request.json",
        "bad-request.json",
        "first.3mf",
        "second.3mf",
    ]);
}

#[test]
fn malformed_and_oversized_fields_fail_closed_and_worker_continues() {
    let fixture = Fixture::new("fields");
    let long_path = "p".repeat(4097);
    let input = [
        "{not-json}\n".to_owned(),
        "{\"request_file\":5,\"source\":\"s\",\"destination\":\"d\"}\n".to_owned(),
        format!("{{\"request_file\":\"{long_path}\",\"source\":\"s\",\"destination\":\"d\"}}\n"),
        job(
            &fixture.request,
            &fixture.source,
            &fixture.output("after-errors.3mf"),
        ),
    ]
    .concat();
    let output = run_worker(input.into_bytes());
    assert!(output.status.success(), "{}", stderr(&output));
    let replies = replies(&output);
    assert_eq!(replies.len(), 4);
    for reply in &replies[..3] {
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].as_str().is_some_and(|s| !s.is_empty()));
    }
    assert_eq!(replies[3], serde_json::from_str::<Value>(RECEIPT).unwrap());
    fixture.assert_no_residue(&[
        "source.3mf",
        "request.json",
        "bad-request.json",
        "after-errors.3mf",
    ]);
}

#[test]
fn eof_without_jobs_exits_cleanly_with_no_output() {
    let output = run_worker(Vec::new());
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).is_empty());
}

#[test]
fn oversized_line_ends_worker_with_framed_error() {
    let oversized = vec![b'{'; LINE_LIMIT + 2];
    let output = run_worker(oversized);
    assert!(!output.status.success());
    let replies = replies(&output);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["ok"], false);
    assert!(replies[0]["error"].as_str().unwrap().contains("8 MB limit"));
    assert!(stderr(&output).contains("native stdio worker terminated"));
}

#[test]
fn job_cap_is_enforced_with_a_framed_stop() {
    let input = "{}\n".repeat(MAX_JOBS + 1);
    let output = run_worker(input.into_bytes());
    assert!(!output.status.success());
    let replies = replies(&output);
    assert_eq!(replies.len(), MAX_JOBS);
    assert!(replies.iter().all(|reply| reply["ok"] == false));
    assert!(stderr(&output).contains("maximum 4096 requests"));
}
