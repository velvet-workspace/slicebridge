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
        || key == "filament_max_volumetric_speed"
        || key.contains("flow_ratio")
        || key == "filament_type"
        || key == "filament_settings_id"
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
                        "filament_settings_id": ["PLA-native"],
                        "cooling_fan": ["60"],
                        "nozzle_diameter": ["0.2"]
                    },
                    "PETG": {
                        "nozzle_temperature": ["245"],
                        "flow_ratio": ["0.97"],
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
                        "nozzle_temperature": ["210"]
                    },
                    "PETG": {"nozzle_temperature": ["245"]}
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
                    "PLA": {"filament_settings_id": ["Installed leaf PLA"], "nozzle_temperature": ["210"]},
                    "PETG": {"filament_settings_id": ["Installed leaf PETG"], "nozzle_temperature": ["245"]}
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
                    "PLA": {"nozzle_temperature": ["210"], "flow_ratio": ["0.98"]},
                    "PETG": {"flow_ratio": ["0.97"]}
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
}
