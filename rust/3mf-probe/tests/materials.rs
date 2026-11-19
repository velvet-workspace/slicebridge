use serde_json::{json, Value};
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_NUMBER: AtomicU64 = AtomicU64::new(0);

struct RequestFile(PathBuf);

impl RequestFile {
    fn new(bytes: &[u8]) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let filename = std::env::temp_dir().join(format!(
            "slicebridge-material-test-{}-{now}-{}",
            std::process::id(),
            TEST_NUMBER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&filename, bytes).unwrap();
        Self(filename)
    }
}

impl Drop for RequestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn cli(command: &str, input: &RequestFile) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg(command)
        .arg(&input.0)
        .output()
        .unwrap();
    let response = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.success(), response)
}

#[test]
fn slot_plan_cli_preserves_each_virtual_slot_and_support_material() {
    let request = RequestFile::new(
        br##"{"filaments":[{"id":9,"type":"PLA","color":"#FFFFFF"},{"id":2,"type":"PVA","color":"#FFFFFF"}]}"##,
    );
    let (ok, result) = cli("--plan-slots", &request);
    assert!(ok);
    assert_eq!(result["ok"], true);
    assert_eq!(result["plan"]["mapping"], json!({"9": 1, "2": 2}));
    assert_eq!(result["plan"]["slots"][0]["type"], "PLA");
    assert_eq!(result["plan"]["slots"][1]["support"], true);
}

#[test]
fn material_map_cli_does_not_change_hardware_owned_settings() {
    let request = RequestFile::new(
        br#"{
          "bundle": {
            "target": "c5",
            "profile": {
              "filament_slots": 4,
              "filament_profiles": {"PLA": "Native PLA", "PETG": "Native PETG"},
              "project_settings": {"nozzle_diameter": ["0.4", "0.4"], "machine_start_gcode": "G28"}
            },
            "material_settings": {
              "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"], "nozzle_diameter": ["0.2"]},
              "PETG": {"nozzle_temperature": ["240"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"], "nozzle_diameter": ["0.6"]}
            }
          },
          "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        }"#,
    );
    let (ok, result) = cli("--map-materials", &request);
    assert!(ok);
    assert_eq!(result["ok"], true);
    assert_eq!(
        result["project_settings"]["nozzle_temperature"],
        json!(["210", "240"])
    );
    assert_eq!(
        result["project_settings"]["nozzle_diameter"],
        json!(["0.4", "0.4"])
    );
    assert_eq!(
        result["project_settings"]["filament_type"],
        json!(["PLA", "PETG"])
    );
    assert_eq!(result["project_settings"]["machine_start_gcode"], "G28");
}

#[test]
fn malformed_requests_fail_with_json_and_nonzero_exit_code() {
    let request = RequestFile::new(b"{invalid");
    let (ok, result) = cli("--map-materials", &request);
    assert!(!ok);
    assert_eq!(result["ok"], false);
}

#[test]
fn oversized_input_rejected_before_json_parse() {
    let request = RequestFile::new(b"{}");
    File::options()
        .write(true)
        .open(&request.0)
        .unwrap()
        .set_len((8 << 20) + 1)
        .unwrap();
    let (ok, result) = cli("--plan-slots", &request);
    assert!(!ok);
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("8 MB limit"));
}

#[test]
fn validate_native_bundle_cli_reports_identity_and_never_emits_settings() {
    let request = RequestFile::new(
        br#"{
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
                "nozzle_diameter": ["0.4"],
                "machine_start_gcode": "START",
                "machine_end_gcode": "END"
              }
            },
            "material_settings": {
              "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"]},
              "PETG": {"nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
            }
          },
          "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        }"#,
    );
    let (ok, result) = cli("--validate-native-bundle", &request);
    assert!(ok);
    assert_eq!(result["ok"], true);
    assert_eq!(result["experimental"], true);
    assert_eq!(result["validation"]["machine"]["target"], "u1");
    assert_eq!(
        result["validation"]["filament_types"],
        json!(["PLA", "PETG"])
    );
    assert!(result.get("project_settings").is_none());
    assert!(result["validation"].get("project_settings").is_none());
}
