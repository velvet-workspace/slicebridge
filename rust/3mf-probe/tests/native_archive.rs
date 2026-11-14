use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use slicebridge_3mf_probe::{convert_native_archive, inspect_package};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

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

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Owns a private scratch directory holding a synthetic source, a request
/// file and any outputs. Dropping it removes everything the test created.
struct Fixture {
    dir: PathBuf,
    source: PathBuf,
    request: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "slicebridge-native-archive-{}-{name}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.3mf");
        write_source(&source);
        let request = dir.join("request.json");
        fs::write(&request, request_value().to_string()).unwrap();
        Fixture {
            dir,
            source,
            request,
        }
    }

    fn output(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn entry_count(&self) -> usize {
        fs::read_dir(&self.dir).unwrap().count()
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

/// Synthetic installed U1 bundle (never a real OEM profile) and five filaments
/// that exactly mirror the source slice info palette.
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

/// (id, material, color) for every filament element in a slice_info document.
fn slice_filaments(xml: &[u8]) -> Vec<(String, String, String)> {
    let mut reader = Reader::from_reader(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(ref start) | Event::Empty(ref start)
                if start.local_name().as_ref() == "filament" =>
            {
                let mut attrs = BTreeMap::new();
                for attribute in start.attributes() {
                    let attribute = attribute.unwrap();
                    let key = attribute.key.as_ref().to_owned();
                    attrs.insert(key, attribute.value.into_owned());
                }
                out.push((
                    attrs["id"].clone(),
                    attrs["type"].clone(),
                    attrs["color"].clone(),
                ));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn assert_refused(fixture: &Fixture, request: &Value, expected: &str) {
    let output = fixture.output("refused.3mf");
    let before = fixture.entry_count();
    let error = convert_native_archive(request, &fixture.source, &output).unwrap_err();
    assert!(error.contains(expected), "{error:?} lacks {expected:?}");
    assert!(!error.contains(SECRET_START), "OEM script leaked: {error}");
    assert!(!output.exists(), "refusal created an output");
    assert_eq!(fixture.entry_count(), before, "refusal left scratch files");
}

#[test]
fn valid_conversion_matches_source_geometry_and_metadata() {
    let fixture = Fixture::new("valid");
    let output = fixture.output("converted.3mf");
    let receipt = convert_native_archive(&request_value(), &fixture.source, &output).unwrap();
    assert_eq!(
        receipt,
        json!({"ok": true, "experimental": true, "removed": 2, "planner": "rust-native"})
    );

    let source_facts = inspect_package(&fixture.source).unwrap();
    let output_facts = inspect_package(&output).unwrap();
    assert_eq!(output_facts.vertices, source_facts.vertices);
    assert_eq!(output_facts.triangles, source_facts.triangles);
    assert_eq!(output_facts.slice_caches, 0);
    assert_eq!(output_facts.printer_model, "Snapmaker U1");

    let source = members(&fixture.source);
    let converted = members(&output);
    let expected_names: Vec<_> = source
        .keys()
        .filter(|name| !name.starts_with("Metadata/plate_1.gcode"))
        .cloned()
        .collect();
    assert_eq!(
        converted.keys().cloned().collect::<Vec<_>>(),
        expected_names
    );
    assert_eq!(converted["3D/3dmodel.model"], source["3D/3dmodel.model"]);
    assert_eq!(converted["Metadata/plate_1.png"], b"THUMBNAIL");

    let palette = slice_filaments(&converted["Metadata/slice_info.config"]);
    let ids: Vec<_> = palette.iter().map(|(id, _, _)| id.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3", "4", "5"]);
    assert_eq!(palette[4].1, "PETG");
    assert_eq!(palette[4].2, "#123456");

    let settings: Value =
        serde_json::from_slice(&converted["Metadata/project_settings.config"]).unwrap();
    assert_eq!(settings["printer_model"], "Snapmaker U1");
    assert_eq!(
        settings["filament_colour"],
        json!(["#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#123456"])
    );
    assert_eq!(
        settings["filament_type"],
        json!(["PLA", "PLA", "PLA", "PLA", "PETG"])
    );
    assert_eq!(settings["machine_start_gcode"], SECRET_START);

    let model_settings =
        String::from_utf8(converted["Metadata/model_settings.config"].clone()).unwrap();
    assert!(model_settings.contains(r#"value="5""#), "{model_settings}");
    assert!(!model_settings.contains(r#"value="7""#), "{model_settings}");
}

#[test]
fn invalid_bundles_and_requests_refuse_without_output() {
    let fixture = Fixture::new("refusals");

    let mut nozzle = request_value();
    nozzle["bundle"]["profile"]["project_settings"]["nozzle_diameter"] = json!(["0.9"]);
    assert_refused(&fixture, &nozzle, "unsupported u1 nozzle");

    let mut mismatched_nozzle = request_value();
    mismatched_nozzle["bundle"]["nozzle"] = json!("0.6");
    assert_refused(
        &fixture,
        &mismatched_nozzle,
        "does not match the verified physical nozzle",
    );

    let mut missing_material = request_value();
    missing_material["bundle"]["material_settings"]["PETG"]
        .as_object_mut()
        .unwrap()
        .remove("nozzle_temperature");
    assert_refused(&fixture, &missing_material, "nozzle_temperature");

    let mut unsupported = request_value();
    unsupported["filaments"][4]["type"] = json!("ABS");
    assert_refused(&fixture, &unsupported, "ABS");

    let mut wrong_color = request_value();
    wrong_color["filaments"][0]["color"] = json!("#FF0001");
    assert_refused(&fixture, &wrong_color, "does not match source slice_info");

    let mut wrong_material = request_value();
    wrong_material["filaments"][4]["type"] = json!("PLA");
    assert_refused(
        &fixture,
        &wrong_material,
        "does not match source slice_info",
    );

    let mut missing_id = request_value();
    missing_id["filaments"][4]["id"] = json!(8);
    assert_refused(&fixture, &missing_id, "absent from source slice_info");

    let mut stale_snapshot = request_value();
    stale_snapshot["source_settings"]["layer_height"] = json!("0.3");
    assert_refused(&fixture, &stale_snapshot, "differs from actual input 3MF");

    let mut no_snapshot = request_value();
    no_snapshot["source_settings"] = json!({});
    assert_refused(&fixture, &no_snapshot, "nonempty JSON object");
}

#[test]
fn existing_destination_is_never_overwritten() {
    let fixture = Fixture::new("existing");
    let output = fixture.output("occupied.3mf");
    fs::write(&output, b"operator bytes").unwrap();
    let error = convert_native_archive(&request_value(), &fixture.source, &output).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(fs::read(&output).unwrap(), b"operator bytes");
}

#[test]
fn source_and_request_bytes_are_immutable() {
    let fixture = Fixture::new("immutable");
    let source_before = fs::read(&fixture.source).unwrap();
    let request_before = fs::read(&fixture.request).unwrap();
    let request: Value = serde_json::from_slice(&request_before).unwrap();
    convert_native_archive(&request, &fixture.source, &fixture.output("out.3mf")).unwrap();
    assert_eq!(fs::read(&fixture.source).unwrap(), source_before);
    assert_eq!(fs::read(&fixture.request).unwrap(), request_before);
}

#[test]
fn repeated_runs_write_fresh_equivalent_outputs() {
    let fixture = Fixture::new("stability");
    let request = request_value();
    let first = fixture.output("first.3mf");
    let second = fixture.output("second.3mf");
    let a = convert_native_archive(&request, &fixture.source, &first).unwrap();
    let b = convert_native_archive(&request, &fixture.source, &second).unwrap();
    assert_eq!(a, b);
    assert_eq!(members(&first), members(&second));
}

#[test]
fn cli_returns_only_compact_receipt_and_refuses_existing_output() {
    let fixture = Fixture::new("cli");
    let output = fixture.output("cli.3mf");
    let run = |destination: &Path| {
        Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
            .arg("--convert-native-archive")
            .arg(&fixture.request)
            .arg(&fixture.source)
            .arg(destination)
            .output()
            .unwrap()
    };

    let ok = run(&output);
    let stdout = String::from_utf8(ok.stdout).unwrap();
    assert!(ok.status.success(), "{stdout}");
    assert!(
        !stdout.contains(SECRET_START),
        "OEM script leaked: {stdout}"
    );
    let receipt: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        receipt,
        json!({"ok": true, "experimental": true, "removed": 2, "planner": "rust-native"})
    );

    let refused = run(&output);
    let stdout = String::from_utf8(refused.stdout).unwrap();
    assert!(!refused.status.success());
    let error: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(error["ok"], false);
    assert!(
        !stdout.contains(SECRET_START),
        "OEM script leaked: {stdout}"
    );
    assert!(inspect_package(&output).is_ok());
}
