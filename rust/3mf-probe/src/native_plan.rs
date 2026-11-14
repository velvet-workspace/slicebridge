//! Experimental, in-memory conversion plan for an explicit installed OEM bundle.
//! Produces the Go rustExplicitPlan JSON keys (profile, slots, slot_count,
//! mapping, source_settings, supports, preserved) without touching any archive,
//! slicer, printer, or installed profile. The Go side is still authoritative.
use crate::materials::{map_native_material_settings, source_slot_plan, validate_native_bundle};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const MAX_PRESERVED: usize = 64;
const MAX_PRESERVED_KEY_BYTES: usize = 128;

// Mirrors Go safePrintParameters in engine/bridge/settings.go. Keep in sync.
const SAFE_PRINT_PARAMETERS: [(&str, f64, f64); 13] = [
    ("layer_height", 0.04, 0.64),
    ("initial_layer_print_height", 0.04, 0.64),
    ("wall_loops", 1.0, 12.0),
    ("top_shell_layers", 0.0, 20.0),
    ("bottom_shell_layers", 0.0, 20.0),
    ("sparse_infill_density", 0.0, 100.0),
    ("support_threshold_angle", 0.0, 90.0),
    ("brim_width", 0.0, 30.0),
    ("skirt_loops", 0.0, 12.0),
    ("skirt_distance", 0.0, 30.0),
    ("elefant_foot_compensation", 0.0, 3.0),
    ("elephant_foot_compensation", 0.0, 3.0),
    ("xy_contour_compensation", -2.0, 2.0),
];

fn parse_number_setting(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 24 {
        return None;
    }
    trimmed
        .strip_suffix('%')
        .unwrap_or(trimmed)
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

// Port of Go profileLayerLimit: every listed value must be one shared number in (0, 1].
fn profile_layer_limit(raw: Option<&Value>) -> Option<f64> {
    let items: Vec<&Value> = match raw? {
        Value::Array(values) => values.iter().collect(),
        single => vec![single],
    };
    let mut first = None;
    for item in items {
        let text = match item {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        let number = parse_number_setting(&text).filter(|n| *n > 0.0 && *n <= 1.0)?;
        if first.is_some_and(|f: f64| (number - f).abs() > 0.000001) {
            return None;
        }
        first.get_or_insert(number);
    }
    first
}

/// Copies explicitly named, bounds-checked source process values into the
/// in-memory OEM settings. Unlike Go's silent skip, every invalid name fails.
fn apply_preserved(
    settings: &mut Map<String, Value>,
    source: &Map<String, Value>,
    names: &[String],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            return Err(format!("preserved key {name:?} is duplicated"));
        }
        let (_, min, max) = SAFE_PRINT_PARAMETERS
            .iter()
            .find(|(key, _, _)| key == name)
            .ok_or_else(|| format!("preserved key {name:?} is not a safe print parameter"))?;
        let original = source
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("preserved key {name:?} must be a string in source_settings"))?;
        if !settings.contains_key(name) {
            return Err(format!(
                "preserved key {name:?} is absent from the OEM profile"
            ));
        }
        let value = parse_number_setting(original)
            .filter(|v| *v >= *min && *v <= *max)
            .ok_or_else(|| format!("preserved key {name:?} is outside its safe range"))?;
        if name == "layer_height" || name == "initial_layer_print_height" {
            let low = profile_layer_limit(settings.get("min_layer_height"));
            let high = profile_layer_limit(settings.get("max_layer_height"));
            if low.is_some_and(|l| value < l) || high.is_some_and(|h| value > h) {
                return Err(format!(
                    "preserved key {name:?} is outside the OEM layer height limits"
                ));
            }
        }
        settings.insert(name.clone(), Value::String(original.to_owned()));
    }
    Ok(())
}

// Mirror Go preservedProfile's automatic safe process-intent selection for
// verified source project_settings, without delegating the decision to Go.
// The result is sorted to keep repeat conversion receipts deterministic.
fn auto_preserved_names(
    target_settings: &Map<String, Value>,
    source_settings: &Map<String, Value>,
) -> Vec<String> {
    let mut accepted = Vec::new();
    for &(key, min, max) in &SAFE_PRINT_PARAMETERS {
        if !target_settings.contains_key(key) {
            continue;
        }
        let Some(original) = source_settings.get(key).and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = parse_number_setting(original) else {
            continue;
        };
        if value < min || value > max {
            continue;
        }
        if key == "layer_height" || key == "initial_layer_print_height" {
            let lower = profile_layer_limit(target_settings.get("min_layer_height"));
            let upper = profile_layer_limit(target_settings.get("max_layer_height"));
            if lower.is_some_and(|floor| value < floor)
                || upper.is_some_and(|ceiling| value > ceiling)
            {
                continue;
            }
        }
        accepted.push(key.to_owned());
    }
    accepted.sort();
    accepted
}

fn preserved_names(request: &Value) -> Result<Vec<String>, String> {
    let Some(raw) = request.get("preserved") else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let values = raw
        .as_array()
        .ok_or_else(|| "preserved must be an array of setting names".to_owned())?;
    if values.len() > MAX_PRESERVED {
        return Err("too many preserved settings".into());
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= MAX_PRESERVED_KEY_BYTES && !s.contains(';'))
                .map(ToOwned::to_owned)
                .ok_or_else(|| "preserved setting key invalid".to_owned())
        })
        .collect()
}

/// Go supports detection: a direct "1" or an override naming enable_support.
fn source_supports(settings: &Map<String, Value>) -> bool {
    if settings.get("enable_support").and_then(Value::as_str) == Some("1") {
        return true;
    }
    settings
        .get("different_settings_to_system")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str().is_some_and(|s| s.contains("enable_support")))
        })
}

// Port of Go alignFlashStudioFourToolSupportFlags for the verified Creator 5
// target. Only an absent list or the known single "0" default is expanded;
// complete or conflicting manufacturer flags are never guessed or remapped.
// Only the installed Flash Studio owner may enable this OEM-specific fix.
fn align_c5_support_flags(settings: &mut Map<String, Value>, count: usize) -> Result<(), String> {
    if settings
        .get("nozzle_diameter")
        .and_then(Value::as_array)
        .map(Vec::len)
        != Some(4)
    {
        return Err("Flashforge Creator 5 requires exactly four physical nozzle entries".into());
    }
    if let Some(raw) = settings.get("filament_is_support") {
        let flags = raw
            .as_array()
            .ok_or_else(|| "Flashforge Creator 5 support flag type is invalid".to_owned())?;
        if flags.len() == count {
            return Ok(());
        }
        let known_default =
            flags.len() == 1 && (flags[0].as_str() == Some("0") || flags[0].as_f64() == Some(0.0));
        if !known_default {
            return Err(format!(
                "Flashforge Creator 5 support flags mismatch {count} material slots"
            ));
        }
    }
    settings.insert(
        "filament_is_support".to_owned(),
        Value::Array(vec![json!("0"); count]),
    );
    Ok(())
}

/// Builds an experimental, Go rustExplicitPlan-compatible plan entirely in memory.
///
/// Request: {"bundle": flattened verified OEM bundle, "filaments": ordered
/// [{"id","color","type"}], "source_settings": object, "preserved": optional
/// array of safe print parameter names}. Omitted preserved triggers Rust's
/// automatic safe source-process selection; explicit [] or null disables it.
/// The input is never mutated.
///
/// Output keys: profile (OEM profile copy with palette-sized filament_slots and
/// map_native_material_settings project_settings), slots, slot_count, mapping,
/// source_settings, supports, preserved.
///
/// Known parity limits versus Go convert.go/targets.go, not reproduced here:
/// Orca-fork compatibility guards, base-profile
/// fallback and Go's silent skipping of unsafe preserved keys. Callers must not
/// treat this plan as an equivalent replacement for the Go path yet.
pub fn build_native_conversion_plan(request: &Value) -> Result<Value, String> {
    let source_settings = request
        .get("source_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "source_settings must be a JSON object".to_owned())?;
    let identity = validate_native_bundle(request)?;
    let source = source_slot_plan(request)?;
    let target = request["bundle"]["target"].as_str().unwrap_or_default();
    let filament_count = source["slots"].as_array().map_or(0, Vec::len);
    // Go sizes QIDI's X-Plus 5 palette to the exact source count; other
    // targets only grow the four-slot OEM palette to fit the source.
    let slot_count = if target == "qidi" {
        filament_count
    } else {
        identity["virtual_filament_slots"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| "virtual filament slot count missing".to_owned())?
    };

    let mut effective = request.clone();
    effective["bundle"]["profile"]["filament_slots"] = json!(slot_count);
    let mapped = map_native_material_settings(&effective)?;
    let mut profile = effective["bundle"]["profile"].clone();
    profile["project_settings"] = mapped["project_settings"].clone();

    let settings = profile["project_settings"]
        .as_object_mut()
        .ok_or_else(|| "profile project_settings must be an object".to_owned())?;
    let slicer = request["bundle"]["slicer"].as_str().unwrap_or_default();
    if target == "c5" && (slicer == "Flash Studio" || slicer == "Flash Studio Desktop") {
        align_c5_support_flags(settings, filament_count.max(4))?;
    }
    // Omission delegates *safe source process intent* selection to Rust.
    // Explicit preserved []/null suppresses automatic selection; an
    // explicit list is validated rather than quietly accepting unsafe keys.
    let preserved = if request.get("preserved").is_none() {
        auto_preserved_names(settings, source_settings)
    } else {
        preserved_names(request)?
    };
    apply_preserved(settings, source_settings, &preserved)?;

    Ok(json!({
        "profile": profile,
        "slots": source["slots"],
        "slot_count": slot_count,
        "mapping": source["mapping"],
        "source_settings": source_settings,
        "supports": source_supports(source_settings),
        "preserved": preserved,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose_project_settings;

    const SECRET_START: &str = "OEM_START_SECRET_GCODE";

    fn bundle_request() -> Value {
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
            "source_settings": {"enable_support": "1", "layer_height": "0.2"}
        })
    }

    fn plan_error(request: &Value) -> String {
        build_native_conversion_plan(request).unwrap_err()
    }

    #[test]
    fn four_pla_one_petg_builds_five_slot_plan_with_source_palette() {
        let request = bundle_request();
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(plan["slot_count"], 5);
        assert_eq!(plan["profile"]["filament_slots"], 5);
        assert_eq!(
            plan["mapping"],
            json!({"1": 1, "2": 2, "3": 3, "4": 4, "7": 5})
        );
        assert_eq!(
            plan["slots"][4],
            json!({"color": "#123456", "type": "PETG"})
        );
        assert_eq!(plan["supports"], true);
        assert_eq!(plan["preserved"], json!(["layer_height"]));
        let settings = &plan["profile"]["project_settings"];
        assert_eq!(
            settings["filament_type"],
            json!(["PLA", "PLA", "PLA", "PLA", "PETG"])
        );
        assert_eq!(settings["filament_settings_id"][4], "Synthetic PETG");
        assert_eq!(
            settings["nozzle_temperature"],
            json!(["210", "210", "210", "210", "245"])
        );
        // Hardware-owned nozzle arrays and installed machine scripts remain OEM-owned.
        assert_eq!(
            settings["nozzle_diameter"],
            json!(["0.4", "0.4", "0.4", "0.4"])
        );
        assert_eq!(settings["machine_start_gcode"], SECRET_START);
    }

    #[test]
    fn plan_composes_into_project_settings_with_source_colors() {
        let plan = build_native_conversion_plan(&bundle_request()).unwrap();
        let composed = compose_project_settings(&plan).unwrap();
        assert_eq!(
            composed["filament_colour"],
            json!(["#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#123456"])
        );
        assert_eq!(composed["printer_model"], "Snapmaker U1");
    }

    #[test]
    fn input_and_original_bundle_are_not_mutated() {
        let request = bundle_request();
        let before = request.clone();
        let _ = build_native_conversion_plan(&request).unwrap();
        assert_eq!(request, before);
    }

    #[test]
    fn named_safe_preserved_key_copies_source_value_after_bounds_checks() {
        let mut request = bundle_request();
        request["preserved"] = json!(["layer_height"]);
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(plan["preserved"], json!(["layer_height"]));
        assert_eq!(plan["profile"]["project_settings"]["layer_height"], "0.2");
    }

    #[test]
    fn missing_critical_native_value_refuses_plan() {
        let mut request = bundle_request();
        request["bundle"]["material_settings"]["PETG"]
            .as_object_mut()
            .unwrap()
            .remove("nozzle_temperature");
        assert!(plan_error(&request).contains("nozzle_temperature"));
    }

    #[test]
    fn unsupported_filament_material_refuses_plan() {
        let mut request = bundle_request();
        request["filaments"][4]["type"] = json!("ABS");
        assert!(plan_error(&request).contains("ABS"));
    }

    #[test]
    fn mixed_oem_nozzles_refuse_plan() {
        let mut request = bundle_request();
        request["bundle"]["profile"]["project_settings"]["nozzle_diameter"] =
            json!(["0.4", "0.4", "0.6", "0.4"]);
        assert!(plan_error(&request).contains("mixed nozzle"));
    }

    #[test]
    fn malformed_source_mapping_refuses_plan() {
        let mut request = bundle_request();
        request["filaments"][0]["id"] = json!("1");
        assert!(plan_error(&request).contains("filament ID"));

        let mut request = bundle_request();
        request["filaments"][1]["id"] = json!(1);
        assert!(plan_error(&request).contains("duplicate source filament ID"));

        let mut request = bundle_request();
        request["filaments"][2]
            .as_object_mut()
            .unwrap()
            .remove("type");
        assert!(plan_error(&request).contains("filament type must be a string"));

        let mut request = bundle_request();
        request["filaments"][2]["type"] = json!(7);
        assert!(plan_error(&request).contains("type"));
    }

    #[test]
    fn missing_or_non_object_source_settings_refuse_plan() {
        let mut request = bundle_request();
        request.as_object_mut().unwrap().remove("source_settings");
        assert!(plan_error(&request).contains("source_settings"));
        let mut request = bundle_request();
        request["source_settings"] = json!("enable_support=1");
        assert!(plan_error(&request).contains("source_settings"));
    }

    #[test]
    fn unsafe_or_unknown_preserved_keys_refuse_plan() {
        let cases = [
            (json!(["bed_temperature"]), "not a safe print parameter"),
            (json!(["layer_height", "layer_height"]), "duplicated"),
            (json!(["wall_loops"]), "must be a string in source_settings"),
            (json!([""]), "invalid"),
            (json!("layer_height"), "must be an array"),
        ];
        for (preserved, expected) in cases {
            let mut request = bundle_request();
            request["preserved"] = preserved;
            assert!(plan_error(&request).contains(expected), "{expected}");
        }

        let mut request = bundle_request();
        request["preserved"] = json!(["layer_height"]);
        request["source_settings"]["layer_height"] = json!("5");
        assert!(plan_error(&request).contains("safe range"));

        let mut request = bundle_request();
        request["preserved"] = json!(["layer_height"]);
        request["source_settings"]["layer_height"] = json!("0.5");
        request["bundle"]["profile"]["project_settings"]["max_layer_height"] = json!(["0.25"]);
        assert!(plan_error(&request).contains("layer height limits"));
    }

    #[test]
    fn auto_preserves_only_valid_safe_process_intent_in_stable_order() {
        let mut request = bundle_request();
        request["bundle"]["profile"]["project_settings"]["wall_loops"] = json!("3");
        request["bundle"]["profile"]["project_settings"]["brim_width"] = json!("0");
        request["source_settings"]["brim_width"] = json!("8");
        request["source_settings"]["wall_loops"] = json!("4");
        request["source_settings"]["machine_start_gcode"] = json!("INJECTED");
        request["source_settings"]["nozzle_temperature"] = json!("999");
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(
            plan["preserved"],
            json!(["brim_width", "layer_height", "wall_loops"])
        );
        assert_eq!(plan["profile"]["project_settings"]["wall_loops"], "4");
        assert_eq!(plan["profile"]["project_settings"]["brim_width"], "8");
        assert_eq!(
            plan["profile"]["project_settings"]["machine_start_gcode"],
            SECRET_START
        );
        assert_eq!(
            request["bundle"]["profile"]["project_settings"]["wall_loops"],
            "3"
        );

        request["source_settings"]["wall_loops"] = json!("200");
        request["source_settings"]["brim_width"] = json!("NaN");
        request["source_settings"]["layer_height"] = json!("0.9");
        let filtered = build_native_conversion_plan(&request).unwrap();
        assert_eq!(filtered["preserved"], json!([]));
        assert_eq!(filtered["profile"]["project_settings"]["wall_loops"], "3");

        request["source_settings"]["layer_height"] = json!("0.24");
        request["bundle"]["profile"]["project_settings"]["max_layer_height"] = json!(["0.2"]);
        let bounded = build_native_conversion_plan(&request).unwrap();
        assert_eq!(bounded["preserved"], json!([]));
    }

    #[test]
    fn explicit_empty_or_null_preserved_disables_rust_auto_selection() {
        let mut request = bundle_request();
        for explicit in [json!([]), Value::Null] {
            request["preserved"] = explicit;
            let plan = build_native_conversion_plan(&request).unwrap();
            assert_eq!(plan["preserved"], json!([]));
        }
    }

    #[test]
    fn supports_false_without_support_settings() {
        let mut request = bundle_request();
        request["source_settings"] = json!({"layer_height": "0.2"});
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(plan["supports"], false);
    }

    // Synthetic Creator 5 bundle: four physical tools, source truncated to
    // the requested filament count. Only identity fields are changed.
    fn c5_request(filament_count: usize) -> Value {
        let mut request = bundle_request();
        request["bundle"]["target"] = json!("c5");
        request["bundle"]["slicer"] = json!("Flash Studio Desktop");
        request["bundle"]["profile"]["name"] = json!("c5");
        request["bundle"]["profile"]["display_name"] = json!("Flashforge Creator 5");
        request["bundle"]["profile"]["printer_model_id"] = json!("Flashforge Creator 5");
        request["bundle"]["profile"]["project_settings"]["printer_model"] =
            json!("Flashforge Creator 5");
        request["filaments"]
            .as_array_mut()
            .unwrap()
            .truncate(filament_count);
        request
    }

    #[test]
    fn c5_missing_support_flags_expand_per_slot_with_four_slot_floor() {
        let plan = build_native_conversion_plan(&c5_request(4)).unwrap();
        assert_eq!(
            plan["profile"]["project_settings"]["filament_is_support"],
            json!(["0", "0", "0", "0"])
        );
        let plan = build_native_conversion_plan(&c5_request(5)).unwrap();
        assert_eq!(
            plan["profile"]["project_settings"]["filament_is_support"],
            json!(["0", "0", "0", "0", "0"])
        );
    }

    #[test]
    fn c5_expands_only_known_zero_default_and_keeps_complete_flags() {
        let mut request = c5_request(4);
        request["bundle"]["profile"]["project_settings"]["filament_is_support"] = json!(["0"]);
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(
            plan["profile"]["project_settings"]["filament_is_support"],
            json!(["0", "0", "0", "0"])
        );

        let mut request = c5_request(4);
        request["bundle"]["profile"]["project_settings"]["filament_is_support"] =
            json!(["0", "1", "0", "1"]);
        let plan = build_native_conversion_plan(&request).unwrap();
        assert_eq!(
            plan["profile"]["project_settings"]["filament_is_support"],
            json!(["0", "1", "0", "1"])
        );
    }

    #[test]
    fn c5_refuses_ambiguous_or_malformed_support_flags() {
        let cases = [
            (json!(["0", "1"]), "mismatch 4 material slots"),
            (json!(["1"]), "mismatch 4 material slots"),
            (json!("0"), "type is invalid"),
        ];
        for (flags, expected) in cases {
            let mut request = c5_request(4);
            request["bundle"]["profile"]["project_settings"]["filament_is_support"] = flags;
            assert!(plan_error(&request).contains(expected), "{expected}");
        }
    }

    #[test]
    fn c5_non_flash_studio_profile_is_not_modified() {
        let mut request = c5_request(4);
        request["bundle"]["slicer"] = json!("OrcaSlicer");
        let plan = build_native_conversion_plan(&request).unwrap();
        assert!(plan["profile"]["project_settings"]
            .get("filament_is_support")
            .is_none());
    }

    #[test]
    fn c5_refuses_physical_nozzle_array_that_is_not_four_entries() {
        let mut request = c5_request(4);
        request["bundle"]["profile"]["project_settings"]["nozzle_diameter"] = json!(["0.4"]);
        assert!(plan_error(&request).contains("exactly four physical nozzle"));
    }

    #[test]
    fn non_c5_targets_never_gain_or_rewrite_support_flags() {
        for (target, model) in [
            ("u1", "Snapmaker U1"),
            ("cc2", "Elegoo Centauri Carbon 2"),
            ("qidi", "X-Plus 5"),
        ] {
            let mut request = bundle_request();
            request["bundle"]["target"] = json!(target);
            request["bundle"]["profile"]["name"] = json!(target);
            request["bundle"]["profile"]["display_name"] = json!(model);
            request["bundle"]["profile"]["printer_model_id"] = json!(model);
            request["bundle"]["profile"]["project_settings"]["printer_model"] = json!(model);
            let plan = build_native_conversion_plan(&request).unwrap();
            assert!(
                plan["profile"]["project_settings"]
                    .get("filament_is_support")
                    .is_none(),
                "{target} gained support flags"
            );

            request["bundle"]["profile"]["project_settings"]["filament_is_support"] =
                json!(["0", "1"]);
            let plan = build_native_conversion_plan(&request).unwrap();
            assert_eq!(
                plan["profile"]["project_settings"]["filament_is_support"],
                json!(["0", "1"]),
                "{target} rewrote support flags"
            );
        }
    }
}
