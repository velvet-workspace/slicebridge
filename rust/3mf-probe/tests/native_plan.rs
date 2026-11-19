use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SECRET_START: &str = "OEM_START_SECRET_GCODE";

fn request_file(name: &str, value: &Value) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "slicebridge-native-plan-{}-{name}.json",
        std::process::id()
    ));
    fs::write(&path, value.to_string()).unwrap();
    path
}

fn request() -> Value {
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
                    "machine_end_gcode": "END"
                }
            },
            "material_settings": {
                "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"]},
                "PETG": {"nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
            }
        },
        "filaments": [
            {"id": 1, "color": "#FF0000", "type": "PLA"},
            {"id": 2, "color": "#00FF00", "type": "PLA"},
            {"id": 3, "color": "#0000FF", "type": "PLA"},
            {"id": 4, "color": "#FFFF00", "type": "PLA"},
            {"id": 7, "color": "#123456", "type": "PETG"}
        ],
        "source_settings": {"enable_support": "0"}
    })
}

fn run_plan(name: &str, value: &Value) -> (bool, String) {
    let path = request_file(name, value);
    let output = Command::new(env!("CARGO_BIN_EXE_slicebridge-3mf-probe"))
        .arg("--plan-native-conversion")
        .arg(&path)
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn plan_cli_returns_receipt_without_oem_scripts() {
    let (ok, stdout) = run_plan("positive", &request());
    assert!(ok, "{stdout}");
    assert!(
        !stdout.contains(SECRET_START),
        "OEM script leaked: {stdout}"
    );
    assert!(!stdout.contains("machine_end_gcode"), "{stdout}");
    let receipt: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(receipt["ok"], true);
    assert_eq!(receipt["experimental"], true);
    assert_eq!(receipt["receipt"]["slot_count"], 5);
    assert_eq!(
        receipt["receipt"]["mapping"],
        json!({"1": 1, "2": 2, "3": 3, "4": 4, "7": 5})
    );
    assert_eq!(receipt["receipt"]["slots"][4]["type"], "PETG");
    assert_eq!(receipt["receipt"]["supports"], false);
    assert!(receipt["receipt"].get("profile").is_none());
}

#[test]
fn plan_cli_fails_closed_with_json_and_no_script_echo() {
    let mut bad = request();
    bad["filaments"][4]["type"] = json!("ABS");
    bad["bundle"]["profile"]["project_settings"]["nozzle_diameter"] = json!(["0.4", "0.6"]);
    let (ok, stdout) = run_plan("negative", &bad);
    assert!(!ok);
    let error: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(error["ok"], false);
    assert!(
        !stdout.contains(SECRET_START),
        "OEM script leaked: {stdout}"
    );
}
