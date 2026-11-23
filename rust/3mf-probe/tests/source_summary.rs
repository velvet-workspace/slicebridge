use slicebridge_3mf_probe::inspect_source_summary;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const MODEL: &[u8] = br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn fixture(slice_info: Option<&[u8]>, project: Option<&[u8]>) -> Fixture {
    let path = std::env::temp_dir().join(format!(
        "slicebridge-summary-{}-{}.3mf",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("[Content_Types].xml", options).unwrap();
    writer.write_all(b"<Types/>").unwrap();
    writer.start_file("3D/3dmodel.model", options).unwrap();
    writer.write_all(MODEL).unwrap();
    if let Some(data) = slice_info {
        writer
            .start_file("Metadata/slice_info.config", options)
            .unwrap();
        writer.write_all(data).unwrap();
    }
    if let Some(data) = project {
        writer
            .start_file("Metadata/project_settings.config", options)
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    Fixture(path)
}

#[test]
fn source_summary_matches_go_pinned_mixed_material_fixture_without_modifying_source() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bambu-mixed-three.3mf");
    let before = fs::read(&path).unwrap();
    let result = inspect_source_summary(&path).unwrap();
    assert_eq!(result["plates"], 1);
    assert_eq!(result["filaments"].as_array().unwrap().len(), 3);
    assert_eq!(result["filaments"][0]["id"], 1);
    assert_eq!(result["filaments"][0]["type"], "PLA");
    assert_eq!(result["filaments"][0]["color"], "#FF0000");
    assert_eq!(result["filaments"][1]["type"], "PETG");
    assert_eq!(result["filaments"][2]["type"], "ABS");
    assert_eq!(result["filaments"][1]["used_m"], 1.0);
    assert_eq!(result["filaments"][1]["used_g"], 3.0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn two_plate_usage_merges_by_id_and_first_palette_wins_like_go() {
    let raw = br##"<config>
        <plate>
          <metadata key="printer_model_id" value="Bambu X"/>
          <filament id="2" type="PETG" color="#00ff00cc" used_m="1.25" used_g="3"/>
        </plate>
        <plate>
          <filament id="2" type="PLA" color="#000000" used_m="0.75" used_g="2"/>
          <filament id="5" type="" color="invalid" used_m="oops" used_g="4"/>
        </plate>
        <filament id="0" type="PLA" color="#ABCDEF"/>
        <filament id="abc" type="PLA" color="#ABCDEF"/>
      </config>"##;
    let source = fixture(Some(raw), None);
    let result = inspect_source_summary(&source.0).unwrap();
    assert_eq!(result["plates"], 2);
    assert_eq!(result["printer_model_id"], "Bambu X");
    assert_eq!(result["filaments"].as_array().unwrap().len(), 2);
    assert_eq!(result["filaments"][0]["id"], 2);
    assert_eq!(result["filaments"][0]["type"], "PETG");
    assert_eq!(result["filaments"][0]["color"], "#00FF00");
    assert_eq!(result["filaments"][0]["used_m"], 2.0);
    assert_eq!(result["filaments"][0]["used_g"], 5.0);
    assert_eq!(result["filaments"][1]["id"], 5);
    assert_eq!(result["filaments"][1]["type"], "PLA");
    assert_eq!(result["filaments"][1]["color"], "#FFFFFF");
    assert_eq!(result["filaments"][1]["used_m"], 0.0);
    assert_eq!(result["filaments"][1]["used_g"], 4.0);
}

#[test]
fn unsliced_project_fallback_preserves_plate_count_and_default_pla() {
    let raw = br#"<config><plate><metadata key="index" value="1"/></plate></config>"#;
    let settings = br##"{"filament_colour":["#aabbcc77","#001122"],"filament_type":["PETG"]}"##;
    let source = fixture(Some(raw), Some(settings));
    let result = inspect_source_summary(&source.0).unwrap();
    assert_eq!(result["plates"], 1);
    assert_eq!(result["filaments"][0]["color"], "#AABBCC");
    assert_eq!(result["filaments"][0]["type"], "PETG");
    assert_eq!(result["filaments"][1]["color"], "#001122");
    assert_eq!(result["filaments"][1]["type"], "PLA");
    assert_eq!(result["filaments"][1]["used_g"], 0.0);
    // encoding/json into a Go []string treats null as an omitted array.
    let null_types = fixture(
        None,
        Some(br##"{"filament_colour":["#112233"],"filament_type":null}"##),
    );
    assert_eq!(
        inspect_source_summary(&null_types.0).unwrap()["filaments"][0]["type"],
        "PLA"
    );
}

#[test]
fn missing_or_bad_metadata_refuses_without_writing_output() {
    let missing = fixture(None, None);
    let before = fs::read(&missing.0).unwrap();
    assert!(inspect_source_summary(&missing.0).is_err());
    assert_eq!(fs::read(&missing.0).unwrap(), before);
    let invalid_xml = fixture(Some(b"<config><plate></config>"), None);
    assert!(inspect_source_summary(&invalid_xml.0).is_err());
    let empty = fixture(Some(b"<config/>"), Some(br#"{"filament_colour":[]}"#));
    assert!(inspect_source_summary(&empty.0).is_err());
}
