//! Read-only Rust pilots for Go sourceSlotPlan and bindNativeFilamentSettings.
//! These functions never choose physical tools, replace filament materials,
//! modify installed presets, or produce machine/printer commands.
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_VIRTUAL_FILAMENTS: usize = 64;
const HARDWARE_OWNED_KEYS: [&str; 3] = [
    "nozzle_diameter",
    "extruder_offset",
    "machine_max_acceleration_extruding",
];

fn filaments(input: &Value) -> Result<&[Value], String> {
    let values = input
        .get("filaments")
        .and_then(Value::as_array)
        .ok_or_else(|| "filaments must be an array".to_owned())?;
    if values.is_empty() || values.len() > MAX_VIRTUAL_FILAMENTS {
        return Err(format!(
            "source has {} filament slots (allowed 1..{MAX_VIRTUAL_FILAMENTS})",
            values.len()
        ));
    }
    Ok(values)
}

fn material_type(filament: &Value) -> Result<&str, String> {
    filament
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "filament type must be a string".to_owned())
}

fn is_support_type(material: &str) -> bool {
    let lower = material.to_lowercase();
    lower.contains("support")
        || lower.contains("breakaway")
        || material.trim().eq_ignore_ascii_case("PVA")
}

/// Port of bridge.sourceSlotPlan; preserves every source filament ID,
/// even when multiple IDs have identical material types and colors.
pub fn source_slot_plan(input: &Value) -> Result<Value, String> {
    let input_filaments = filaments(input)?;
    let mut slots = Vec::with_capacity(input_filaments.len());
    let mut mapping = BTreeMap::new();

    for filament in input_filaments {
        let id = filament
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| "filament ID must be an integer".to_owned())?;
        if id <= 0 {
            return Err(format!("invalid source filament ID {id}"));
        }
        // Go allocates the ID mapping before checking this filament's color
        // and material. Match its duplicate-ID error precedence exactly.
        if mapping.insert(id.to_string(), slots.len() + 1).is_some() {
            return Err(format!("duplicate source filament ID {id}"));
        }
        let color = filament
            .get("color")
            .and_then(Value::as_str)
            .ok_or_else(|| "filament color must be a string".to_owned())?;
        let material = material_type(filament)?;
        if color.is_empty() || material.is_empty() {
            return Err(format!("missing source color/material in filament {id}"));
        }
        // Go's converter.Slot serializes Support=false with omitempty.
        let slot = if is_support_type(material) {
            json!({"color": color, "type": material, "support": true})
        } else {
            json!({"color": color, "type": material})
        };
        slots.push(slot);
    }

    Ok(json!({"slots": slots, "mapping": mapping}))
}

fn is_critical_key(key: &str) -> bool {
    key.starts_with("nozzle_temperature")
        || key.starts_with("bed_temperature")
        || key.ends_with("_plate_temp")
        || key.contains("_plate_temp_")
        || key == "filament_max_volumetric_speed"
        || key.contains("flow_ratio")
        || key == "filament_type"
        || key == "filament_settings_id"
}

fn finite_positive_number(value: &Value) -> Option<f64> {
    match value {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
    .filter(|v| v.is_finite() && *v > 0.0)
}

fn native_material_safety_value(preset: &Map<String, Value>, key: &str) -> bool {
    preset
        .get(key)
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(finite_positive_number)
        .is_some()
}

fn bundle_object(input: &Value) -> Result<&Map<String, Value>, String> {
    input
        .get("bundle")
        .and_then(Value::as_object)
        .ok_or_else(|| "bundle must be an object".to_owned())
}

/// Port of bridge.bindNativeFilamentSettings operating on an in-memory JSON
/// snapshot of an installed targetBundle. Output is solely project_settings;
/// hardware keys remain unchanged, missing critical fields fail closed.
pub fn map_native_material_settings(input: &Value) -> Result<Value, String> {
    let bundle = bundle_object(input)?;
    let profile = bundle
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "bundle.profile must be an object".to_owned())?;
    let slots = profile
        .get("filament_slots")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "profile filament_slots must be a nonnegative integer".to_owned())?;
    let profiles = profile
        .get("filament_profiles")
        .and_then(Value::as_object)
        .ok_or_else(|| "profile filament_profiles must be an object".to_owned())?;
    let presets_by_type = bundle
        .get("material_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "bundle.material_settings must be an object".to_owned())?;
    let target = bundle
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    // Production Go only substitutes the validated installed-name mapping
    // for QIDI's exact X-Plus 5 profile. Other slicers own their preset IDs
    // through their actual material leaf values and may reject missing IDs.
    let qidi_name_mapping = target == "qidi"
        && profile.get("printer_model_id").and_then(Value::as_str) == Some("X-Plus 5");

    let input_filaments = filaments(input)?;
    if input_filaments.len() > slots {
        return Err("invalid number of source filaments".to_owned());
    }
    let mut chosen_presets: Vec<&Map<String, Value>> = Vec::with_capacity(input_filaments.len());
    let mut names: Vec<&str> = Vec::with_capacity(input_filaments.len());
    let mut native_profile_ids: Vec<&str> = Vec::with_capacity(input_filaments.len());
    for filament in input_filaments {
        let material = material_type(filament)?;
        let profile_name = profiles.get(material).and_then(Value::as_str).unwrap_or("");
        let Some(preset) = presets_by_type.get(material).and_then(Value::as_object) else {
            return Err(format!(
                "no installed {target} filament preset for material type {material:?}"
            ));
        };
        if profile_name.is_empty() || preset.is_empty() {
            return Err(format!(
                "no installed {target} filament preset for material type {material:?}"
            ));
        }
        names.push(material.trim());
        native_profile_ids.push(profile_name);
        chosen_presets.push(preset);
    }

    // The union of present OEM keys cannot detect a critical value missing
    // from every selected material. Refuse before copying any settings.
    for (slot, preset) in chosen_presets.iter().enumerate() {
        for key in ["nozzle_temperature", "filament_max_volumetric_speed"] {
            if !native_material_safety_value(preset, key) {
                return Err(format!(
                    "missing critical native material setting {key:?} in source slot {}",
                    slot + 1
                ));
            }
        }
        if !native_material_safety_value(preset, "filament_flow_ratio")
            && !native_material_safety_value(preset, "flow_ratio")
        {
            return Err(format!("missing critical native material setting \"filament_flow_ratio/flow_ratio\" in source slot {}", slot + 1));
        }
    }

    let mut settings = profile
        .get("project_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "profile project_settings must be an object".to_owned())?
        .clone();

    let mut keys = BTreeSet::new();
    for preset in &chosen_presets {
        for (key, value) in *preset {
            if value.as_array().is_some_and(|array| !array.is_empty()) {
                keys.insert(key.as_str());
            }
        }
    }

    for key in keys {
        // Native target resolution supplies one verified material preset name
        // per material type. Some installed vendor filament JSON snapshots
        // omit filament_settings_id or store an empty placeholder. The
        // profile mapping, not a guessed PLA default, owns this value.
        if qidi_name_mapping && key == "filament_settings_id" {
            continue;
        }
        if HARDWARE_OWNED_KEYS.contains(&key) {
            continue;
        }
        let mut values = Vec::with_capacity(chosen_presets.len());
        let mut missing = false;
        for preset in &chosen_presets {
            match preset.get(key).and_then(Value::as_array) {
                Some(items) if !items.is_empty() => values.push(items[0].clone()),
                _ => {
                    missing = true;
                    break;
                }
            }
        }
        if missing {
            if is_critical_key(key) {
                return Err(format!("missing critical native material setting {key:?}"));
            }
            // A partial optional preset cannot be copied to the other slots.
            settings.remove(key);
        } else {
            settings.insert(key.to_owned(), Value::Array(values));
        }
    }
    if qidi_name_mapping {
        settings.insert(
            "filament_settings_id".to_owned(),
            Value::Array(
                native_profile_ids
                    .into_iter()
                    .map(|name| Value::String(name.to_owned()))
                    .collect(),
            ),
        );
    }
    // Match Go: filament_type always comes from the source's ordered slots.
    settings.insert(
        "filament_type".to_owned(),
        Value::Array(
            names
                .into_iter()
                .map(|material| Value::String(material.to_owned()))
                .collect(),
        ),
    );

    Ok(json!({"project_settings": settings}))
}

/// Model identity and physical nozzles per target, as fixed in Go targets.go.
fn target_identity(target: &str) -> Option<(&'static str, &'static [f64])> {
    match target {
        "u1" => Some(("Snapmaker U1", &[0.2, 0.4, 0.6, 0.8])),
        "c5" => Some(("Flashforge Creator 5", &[0.25, 0.4, 0.6, 0.8])),
        "cc2" => Some(("Elegoo Centauri Carbon 2", &[0.2, 0.4, 0.6, 0.8])),
        "qidi" => Some(("X-Plus 5", &[0.2, 0.4, 0.6, 0.8])),
        _ => None,
    }
}

/// Validates a flattened installed OEM bundle without converting anything.
/// Material safety is delegated to map_native_material_settings so the rules
/// exist in one place. Returns machine and material identity only.
pub fn validate_native_bundle(input: &Value) -> Result<Value, String> {
    let bundle = bundle_object(input)?;
    let target = bundle
        .get("target")
        .and_then(Value::as_str)
        .filter(|target| !target.is_empty())
        .ok_or_else(|| "bundle.target must be a nonempty string".to_owned())?;
    let (model_id, nozzles) =
        target_identity(target).ok_or_else(|| format!("unsupported native target {target:?}"))?;
    let profile = bundle
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "bundle.profile must be an object".to_owned())?;
    if profile.get("name").and_then(Value::as_str) != Some(target) {
        return Err(format!("profile name must match target {target:?}"));
    }
    if profile.get("display_name").and_then(Value::as_str) != Some(model_id) {
        return Err(format!("profile display_name must be {model_id:?}"));
    }
    if profile.get("printer_model_id").and_then(Value::as_str) != Some(model_id) {
        return Err(format!("profile printer_model_id must be {model_id:?}"));
    }
    if profile.get("filament_slots").and_then(Value::as_u64) != Some(4) {
        return Err("raw OEM profile filament_slots must be 4".to_owned());
    }
    if !profile
        .get("filament_profiles")
        .and_then(Value::as_object)
        .is_some_and(|profiles| !profiles.is_empty())
        || !bundle
            .get("material_settings")
            .and_then(Value::as_object)
            .is_some_and(|settings| !settings.is_empty())
    {
        return Err("installed OEM material profiles must be nonempty".into());
    }
    let settings = profile
        .get("project_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "profile project_settings must be an object".to_owned())?;
    if settings.get("printer_model").and_then(Value::as_str) != Some(model_id) {
        return Err(format!(
            "project_settings printer_model must be {model_id:?}"
        ));
    }
    for key in ["machine_start_gcode", "machine_end_gcode"] {
        if !settings.get(key).is_some_and(Value::is_string) {
            return Err(format!("project_settings {key} must be a string"));
        }
    }
    // Go's nozzleFromRaw refuses mixed nozzle arrays and >16 entries.
    // A single matching element is not sufficient to prove machine identity.
    let raw_nozzles = settings
        .get("nozzle_diameter")
        .ok_or_else(|| "project_settings nozzle_diameter is required".to_owned())?;
    let dimensions: &[Value] = match raw_nozzles {
        Value::Array(values) => values,
        single => std::slice::from_ref(single),
    };
    if dimensions.is_empty() || dimensions.len() > 16 {
        return Err("project_settings nozzle_diameter must contain 1..16 entries".into());
    }
    let mut physical = None;
    for value in dimensions {
        let number = finite_positive_number(value).ok_or_else(|| {
            "project_settings nozzle_diameter must contain finite numbers".to_owned()
        })?;
        let found = nozzles
            .iter()
            .copied()
            .find(|allowed| (allowed - number).abs() < 1e-6)
            .ok_or_else(|| format!("unsupported {target} nozzle diameter {number}"))?;
        if physical.is_some_and(|previous| previous != found) {
            return Err("mixed nozzle diameters are not supported".into());
        }
        physical = Some(found);
    }
    let physical = physical.ok_or_else(|| "missing physical nozzle".to_owned())?;
    // Go's validTargetNozzle accepts exact canonical target strings.
    let bundled_nozzle = bundle
        .get("nozzle")
        .and_then(Value::as_str)
        .ok_or_else(|| "bundle.nozzle must be a canonical string".to_owned())?;
    if !nozzles
        .iter()
        .any(|allowed| allowed.to_string() == bundled_nozzle)
        || bundled_nozzle != physical.to_string()
    {
        return Err("bundle.nozzle does not match the verified physical nozzle".into());
    }

    // The UI's saved physical nozzle selection is a separate authority
    // from the installed OEM bundle. If a caller supplies that selection,
    // reject a mismatched profile rather than silently converting with a
    // different physical tool size. A legacy source-matched caller may omit
    // this field; that mode still requires an independently verified bundle.
    if let Some(selected) = input.get("requested_nozzle") {
        let selected = selected
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "requested_nozzle must be a nonempty canonical string".to_owned())?;
        if selected != bundled_nozzle {
            return Err(format!(
                "requested physical nozzle {selected:?} does not match installed OEM bundle nozzle {bundled_nozzle:?}"
            ));
        }
    }

    // A raw installed OEM bundle has four declared virtual slots. Source
    // projects can have more virtual materials than physical tools, without
    // modifying the actual installed OEM profile or printer configuration.
    let virtual_slots = filaments(input)?.len().max(4);
    let mut effective = input.clone();
    effective["bundle"]["profile"]["filament_slots"] = json!(virtual_slots);
    let mapped = map_native_material_settings(&effective)?;
    Ok(json!({
        "machine": {"target": target, "model_id": model_id, "nozzle_diameter": physical},
        "filament_types": mapped["project_settings"]["filament_type"],
        "virtual_filament_slots": virtual_slots,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_preserve_distinct_ids_and_support_identity() {
        let input = json!({"filaments": [
            {"id": 8, "color": "#FFFFFF", "type": "PLA"},
            {"id": 3, "color": "#FFFFFF", "type": "PLA"},
            {"id": 12, "color": "#FFFF00", "type": "PVA"}
        ]});
        let output = source_slot_plan(&input).unwrap();
        assert_eq!(output["mapping"], json!({"8": 1, "3": 2, "12": 3}));
        assert_eq!(output["slots"][0]["type"], "PLA");
        assert!(output["slots"][0].get("support").is_none());
        assert_eq!(output["slots"][2]["support"], true);
    }

    #[test]
    fn slot_mapping_refuses_duplicates_and_empty_colors() {
        let duplicates = json!({"filaments": [
            {"id": 1, "color": "#000000", "type": "PLA"},
            {"id": 1, "color": "#FFFFFF", "type": "PLA"}
        ]});
        assert!(source_slot_plan(&duplicates)
            .unwrap_err()
            .contains("duplicate"));
        let missing = json!({"filaments": [{"id": 1, "color": "", "type": "PLA"}]});
        assert!(source_slot_plan(&missing)
            .unwrap_err()
            .contains("missing source"));
        let duplicate_with_missing_color = json!({"filaments": [
            {"id": 1, "color": "#FFFFFF", "type": "PLA"},
            {"id": 1, "color": "", "type": "PETG"}
        ]});
        assert!(source_slot_plan(&duplicate_with_missing_color)
            .unwrap_err()
            .contains("duplicate source filament ID"));
        assert!(source_slot_plan(&json!({"filaments": []})).is_err());
    }

    #[test]
    fn material_settings_bind_all_slots_and_leave_hardware_owned_values() {
        let source = json!({
            "bundle": {
                "target": "c5",
                "profile": {
                    "filament_slots": 4,
                    "filament_profiles": {"PLA": "PLA-native", "PETG": "PETG-native"},
                    "project_settings": {
                        "nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"],
                        "machine_start_gcode": "G28",
                        "cooling_fan": ["60"]
                    }
                },
                "material_settings": {
                    "PLA": {
                        "nozzle_temperature": ["210"],
                        "flow_ratio": ["0.98"],
                        "filament_max_volumetric_speed": ["18"],
                        "filament_settings_id": ["PLA-native"],
                        "cooling_fan": ["60"],
                        "nozzle_diameter": ["0.2"]
                    },
                    "PETG": {
                        "nozzle_temperature": ["245"],
                        "flow_ratio": ["0.97"],
                        "filament_max_volumetric_speed": ["12"],
                        "filament_settings_id": ["PETG-native"],
                        "nozzle_diameter": ["0.6"]
                    }
                }
            },
            "filaments": [
                {"id": 1, "type": "PLA"},
                {"id": 2, "type": "PETG"}
            ]
        });
        let value = map_native_material_settings(&source).unwrap();
        let settings = &value["project_settings"];
        assert_eq!(settings["nozzle_temperature"], json!(["210", "245"]));
        assert_eq!(settings["flow_ratio"], json!(["0.98", "0.97"]));
        assert_eq!(settings["filament_type"], json!(["PLA", "PETG"]));
        assert_eq!(
            settings["nozzle_diameter"],
            json!(["0.4", "0.4", "0.4", "0.4"])
        );
        assert_eq!(settings["machine_start_gcode"], "G28");
        assert!(settings.get("cooling_fan").is_none());
    }

    #[test]
    fn qidi_native_filament_profile_id_comes_from_verified_map_when_preset_omits_id() {
        let mut req = json!({
            "bundle": {
                "target": "qidi",
                "profile": {
                    "printer_model_id": "X-Plus 5",
                    "filament_slots": 4,
                    "filament_profiles": {"PLA": "Verified U1 PLA", "PETG": "Verified U1 PETG"},
                    "project_settings": {"nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"]}
                },
                "material_settings": {
                    "PLA": {
                        "filament_settings_id": [""],
                        "nozzle_temperature": ["210"],
                        "flow_ratio": ["0.98"],
                        "filament_max_volumetric_speed": ["18"]
                    },
                    "PETG": {"nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
                }
            },
            "filaments": [
                {"id": 1, "type": "PLA"},
                {"id": 2, "type": "PETG"}
            ]
        });
        let result = map_native_material_settings(&req).unwrap();
        assert_eq!(
            result["project_settings"]["filament_settings_id"],
            json!(["Verified U1 PLA", "Verified U1 PETG"])
        );
        assert_eq!(
            result["project_settings"]["nozzle_temperature"],
            json!(["210", "245"])
        );
        assert!(req["bundle"]["material_settings"]["PETG"]
            .get("filament_settings_id")
            .is_none());
        req["bundle"]["profile"]["filament_profiles"]
            .as_object_mut()
            .unwrap()
            .remove("PETG");
        assert!(map_native_material_settings(&req)
            .unwrap_err()
            .contains("no installed"));
    }

    #[test]
    fn non_qidi_keeps_real_installed_leaf_ids_and_refuses_partial_material_ids() {
        let mut req = json!({
            "bundle": {
                "target": "u1",
                "profile": {
                    "printer_model_id": "Snapmaker U1",
                    "filament_slots": 4,
                    "filament_profiles": {"PLA": "Profile-map name PLA", "PETG": "Profile-map name PETG"},
                    "project_settings": {"machine_start_gcode": "G28"}
                },
                "material_settings": {
                    "PLA": {"filament_settings_id": ["Installed leaf PLA"], "nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"]},
                    "PETG": {"filament_settings_id": ["Installed leaf PETG"], "nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
                }
            },
            "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        });
        let output = map_native_material_settings(&req).unwrap();
        assert_eq!(
            output["project_settings"]["filament_settings_id"],
            json!(["Installed leaf PLA", "Installed leaf PETG"])
        );
        assert_eq!(
            output["project_settings"]["nozzle_temperature"],
            json!(["210", "245"])
        );
        req["bundle"]["material_settings"]["PETG"]
            .as_object_mut()
            .unwrap()
            .remove("filament_settings_id");
        let error = map_native_material_settings(&req).unwrap_err();
        assert!(
            error.contains("missing critical native material setting"),
            "incomplete non-QIDI material IDs must fail closed: {error}"
        );
    }

    #[test]
    fn all_missing_or_blank_installed_material_values_fail_closed() {
        let original = json!({
            "bundle": {
                "target": "c5",
                "profile": {
                    "filament_slots": 2,
                    "filament_profiles": {"PLA": "Native PLA", "PETG": "Native PETG"},
                    "project_settings": {}
                },
                "material_settings": {
                    "PLA": {
                        "nozzle_temperature": ["210"],
                        "flow_ratio": ["0.98"],
                        "filament_max_volumetric_speed": ["18"],
                        "filament_settings_id": ["Native PLA"]
                    },
                    "PETG": {
                        "nozzle_temperature": ["245"],
                        "flow_ratio": ["0.97"],
                        "filament_max_volumetric_speed": ["12"],
                        "filament_settings_id": ["Native PETG"]
                    }
                }
            },
            "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        });
        assert!(map_native_material_settings(&original).is_ok());
        for key in [
            "nozzle_temperature",
            "flow_ratio",
            "filament_max_volumetric_speed",
        ] {
            let mut missing = original.clone();
            for material in ["PLA", "PETG"] {
                missing["bundle"]["material_settings"][material]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
            }
            let error = map_native_material_settings(&missing).unwrap_err();
            assert!(
                error.contains(key),
                "missing all {key} fields must fail: {error}"
            );
        }
        for bad_value in [
            json!(""),
            json!("  "),
            Value::Null,
            json!(0),
            json!("0"),
            json!("-5"),
            json!("abc"),
            json!("NaN"),
            json!("Infinity"),
        ] {
            let mut invalid = original.clone();
            invalid["bundle"]["material_settings"]["PLA"]["nozzle_temperature"] =
                json!([bad_value]);
            let error = map_native_material_settings(&invalid).unwrap_err();
            assert!(
                error.contains("nozzle_temperature"),
                "blank or null temperature accepted: {error}"
            );
        }
    }

    #[test]
    fn partial_installed_plate_temperature_never_falls_back_to_wrong_material() {
        let mut req = json!({
            "bundle": {
                "target": "u1",
                "profile": {
                    "filament_slots": 2,
                    "filament_profiles": {"PLA": "PLA-native", "PETG": "PETG-native"},
                    "project_settings": {"textured_plate_temp": ["999"]}
                },
                "material_settings": {
                    "PLA": {"nozzle_temperature": ["210"], "filament_flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"], "textured_plate_temp": ["60"]},
                    "PETG": {"nozzle_temperature": ["245"], "filament_flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
                }
            },
            "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        });
        let error = map_native_material_settings(&req).unwrap_err();
        assert!(
            error.contains("textured_plate_temp"),
            "unsafe fallback accepted: {error}"
        );
        req["bundle"]["material_settings"]["PETG"]["textured_plate_temp"] = json!(["75"]);
        let output = map_native_material_settings(&req).unwrap();
        assert_eq!(
            output["project_settings"]["textured_plate_temp"],
            json!(["60", "75"])
        );
    }

    #[test]
    fn mapping_rejects_unknown_material_and_missing_critical_values() {
        let mut req = json!({
            "bundle": {
                "target": "c5",
                "profile": {
                    "filament_slots": 4,
                    "filament_profiles": {"PLA": "PLA-native", "PETG": "PETG-native"},
                    "project_settings": {}
                },
                "material_settings": {
                    "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"]},
                    "PETG": {"flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"]}
                }
            },
            "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        });
        assert!(map_native_material_settings(&req)
            .unwrap_err()
            .contains("nozzle_temperature"));
        req["filaments"][1]["type"] = json!("PLA-CF");
        assert!(map_native_material_settings(&req)
            .unwrap_err()
            .contains("PLA-CF"));
    }

    fn synthetic_bundle() -> Value {
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
                        "machine_start_gcode": "START",
                        "machine_end_gcode": "END"
                    }
                },
                "material_settings": {
                    "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"], "filament_max_volumetric_speed": ["18"], "filament_settings_id": ["Synthetic PLA"]},
                    "PETG": {"nozzle_temperature": ["245"], "flow_ratio": ["0.97"], "filament_max_volumetric_speed": ["12"], "filament_settings_id": ["Synthetic PETG"]}
                }
            },
            "filaments": [{"id": 1, "type": "PLA"}, {"id": 2, "type": "PETG"}]
        })
    }

    #[test]
    fn valid_two_material_bundle_reports_identity_without_conversion() {
        let output = validate_native_bundle(&synthetic_bundle()).unwrap();
        assert_eq!(output["machine"]["model_id"], "Snapmaker U1");
        assert_eq!(output["machine"]["nozzle_diameter"], 0.4);
        assert_eq!(output["filament_types"], json!(["PLA", "PETG"]));
        assert!(output.get("project_settings").is_none());
    }

    #[test]
    fn bundle_refuses_missing_model_identity_and_wrong_nozzle() {
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]
            .as_object_mut()
            .unwrap()
            .remove("printer_model_id");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("printer_model_id"));
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["project_settings"]["nozzle_diameter"] = json!(["0.3"]);
        assert!(validate_native_bundle(&req).unwrap_err().contains("nozzle"));
    }

    #[test]
    fn bundle_refuses_absent_nozzle_and_missing_machine_scripts() {
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["project_settings"]
            .as_object_mut()
            .unwrap()
            .remove("nozzle_diameter");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("nozzle_diameter"));
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["project_settings"]
            .as_object_mut()
            .unwrap()
            .remove("machine_end_gcode");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("machine_end_gcode"));
    }

    #[test]
    fn bundle_refuses_unsafe_missing_material_settings() {
        let mut req = synthetic_bundle();
        req["bundle"]["material_settings"]["PETG"]
            .as_object_mut()
            .unwrap()
            .remove("filament_max_volumetric_speed");
        let error = validate_native_bundle(&req).unwrap_err();
        assert!(
            error.contains("filament_max_volumetric_speed"),
            "unsafe material accepted: {error}"
        );
    }

    #[test]
    fn bundle_refuses_wrong_name_display_and_mixed_nozzles() {
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["name"] = json!("qidi");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("profile name"));
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["display_name"] = json!("invented");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("display_name"));
        let mut req = synthetic_bundle();
        req["bundle"]["profile"]["project_settings"]["nozzle_diameter"] =
            json!(["0.4", "0.4", "0.6", "0.4"]);
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("mixed nozzle"));
        let mut req = synthetic_bundle();
        req["bundle"]["nozzle"] = json!("00.4");
        assert!(validate_native_bundle(&req)
            .unwrap_err()
            .contains("bundle.nozzle"));
    }

    #[test]
    fn bundle_validates_five_virtual_material_slots_without_changing_raw_machine() {
        let mut req = synthetic_bundle();
        req["filaments"] = json!([
            {"id": 1, "type": "PLA"}, {"id": 2, "type": "PLA"},
            {"id": 3, "type": "PETG"}, {"id": 4, "type": "PLA"},
            {"id": 5, "type": "PETG"}
        ]);
        assert_eq!(req["bundle"]["profile"]["filament_slots"], 4);
        let reply = validate_native_bundle(&req).unwrap();
        assert_eq!(reply["virtual_filament_slots"], 5);
        assert_eq!(
            reply["filament_types"],
            json!(["PLA", "PLA", "PETG", "PLA", "PETG"])
        );
        assert_eq!(req["bundle"]["profile"]["filament_slots"], 4);
    }

    #[test]
    fn selected_physical_nozzle_must_match_verified_bundle() {
        let mut request = synthetic_bundle();
        request["requested_nozzle"] = json!("0.4");
        assert!(validate_native_bundle(&request).is_ok());
        for selected in [json!("0.6"), json!("00.4"), json!(null), json!(0.4)] {
            request["requested_nozzle"] = selected;
            let error = validate_native_bundle(&request).unwrap_err();
            assert!(error.contains("nozzle"), "mismatch accepted: {error}");
        }
    }

    #[test]
    fn bundle_refuses_source_material_not_in_real_preset() {
        let mut req = synthetic_bundle();
        req["filaments"][1]["type"] = json!("ABS");
        assert!(validate_native_bundle(&req).unwrap_err().contains("ABS"));
    }
}
