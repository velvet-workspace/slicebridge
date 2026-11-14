//! Experimental, pure Rust equivalent of Go converter.buildProjectSettings.
//! No installed profile lookup, 3MF archive creation or printer interaction.
use serde_json::{Map, Value};

const MAX_SLOTS: usize = 64;
const CARRIED_SUPPORT_KEYS: [&str; 4] = [
    "support_type",
    "support_style",
    "support_threshold_angle",
    "support_on_build_plate_only",
];

fn required_object<'a>(input: &'a Value, key: &str) -> Result<&'a Map<String, Value>, String> {
    input
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{key} must be a JSON object"))
}

fn profile_name(profile: &Map<String, Value>, material: &str) -> String {
    profile
        .get("filament_profiles")
        .and_then(Value::as_object)
        .and_then(|names| names.get(material))
        .and_then(Value::as_str)
        .or_else(|| {
            profile
                .get("default_filament_profile")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_owned()
}

fn resize_list(items: &[Value], len: usize) -> Vec<Value> {
    let mut result = items.iter().take(len).cloned().collect::<Vec<_>>();
    if result.len() < len {
        let last = items.last().cloned().expect("nonempty input");
        result.resize(len, last);
    }
    result
}

/// Compose the JSON map written to Metadata/project_settings.config.
/// Equivalent to Go's copyBaseline, applySlotArrays, normalizeFilamentArrays,
/// applySupportSettings; no other conversion phases are included.
///
/// Input schema: {"profile": printer.Profile, "slots": [converter.Slot],
/// "slot_count": integer, "supports": boolean, "source_settings": object}.
/// Arrays, support overrides and all unrelated hardware values follow Go's
/// established behavior. This operation is read-only and cannot select an
/// installed printer or filament on its own.
pub fn compose_project_settings(input: &Value) -> Result<Value, String> {
    let profile = required_object(input, "profile")?;
    let baseline = profile
        .get("project_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "profile.project_settings must be an object".to_owned())?;
    if baseline.is_empty() {
        return Err("profile has no baseline project settings".into());
    }
    let slot_count = input
        .get("slot_count")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "slot_count must be a positive integer".to_owned())?;
    if !(1..=MAX_SLOTS).contains(&slot_count) {
        return Err("slot_count exceeds 1..64 supported range".into());
    }
    let slots = input
        .get("slots")
        .and_then(Value::as_array)
        .ok_or_else(|| "slots must be an array".to_owned())?;
    if slots.len() > slot_count {
        return Err("more source slots than target printer slots".into());
    }
    let supports = input
        .get("supports")
        .and_then(Value::as_bool)
        .ok_or_else(|| "supports must be a boolean".to_owned())?;
    let source = required_object(input, "source_settings")?;

    // Never change caller-supplied profile JSON.
    let mut cfg = baseline.clone();
    let mut colors = Vec::with_capacity(slot_count);
    let mut types = Vec::with_capacity(slot_count);
    let mut settings_ids = Vec::with_capacity(slot_count);

    for index in 0..slot_count {
        if let Some(slot) = slots.get(index) {
            let color = slot
                .get("color")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("slot {index} color must be a string"))?;
            let material = slot
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("slot {index} type must be a string"))?;
            let override_id = slot.get("profile").and_then(Value::as_str).unwrap_or("");
            colors.push(Value::String(color.to_owned()));
            types.push(Value::String(material.to_owned()));
            settings_ids.push(Value::String(if override_id.is_empty() {
                profile_name(profile, material)
            } else {
                override_id.to_owned()
            }));
        } else {
            colors.push(Value::String("#FFFFFF".to_owned()));
            types.push(Value::String("PLA".to_owned()));
            settings_ids.push(Value::String(profile_name(profile, "PLA")));
        }
    }
    cfg.insert("filament_colour".to_owned(), Value::Array(colors));
    cfg.insert("filament_type".to_owned(), Value::Array(types));
    cfg.insert(
        "filament_settings_id".to_owned(),
        Value::Array(settings_ids),
    );

    // Go normalizes every nonempty filament_* array to the slot count and
    // flush_volumes_matrix to the square of that count, repeating the last
    // element when a baseline has fewer entries.
    for (key, value) in &mut cfg {
        let Some(items) = value.as_array() else {
            continue;
        };
        if items.is_empty() {
            continue;
        }
        let desired = if key.len() > 9 && key.starts_with("filament_") {
            Some(slot_count)
        } else if key == "flush_volumes_matrix" {
            Some(slot_count * slot_count)
        } else {
            None
        };
        if let Some(len) = desired {
            *value = Value::Array(resize_list(items, len));
        }
    }

    if !supports {
        cfg.insert("enable_support".to_owned(), Value::String("0".to_owned()));
        cfg.remove("different_settings_to_system");
    } else {
        cfg.insert("enable_support".to_owned(), Value::String("1".to_owned()));
        let mut marked = "enable_support".to_owned();
        for key in CARRIED_SUPPORT_KEYS {
            if let Some(value) = source.get(key) {
                cfg.insert(key.to_owned(), value.clone());
                marked.push(';');
                marked.push_str(key);
            }
        }
        let mut differences = vec![Value::String(String::new()); slot_count + 2];
        differences[0] = Value::String(marked);
        cfg.insert(
            "different_settings_to_system".to_owned(),
            Value::Array(differences),
        );
    }

    Ok(Value::Object(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(supports: bool) -> Value {
        json!({
            "profile": {
                "filament_profiles": {"PLA": "PLA Native", "PETG": "PETG Native"},
                "default_filament_profile": "Default Native",
                "project_settings": {
                    "filament_flow_ratio": ["0.99"],
                    "filament_density": ["1.2", "1.3", "1.4", "1.5", "1.6"],
                    "flush_volumes_matrix": [0, 1],
                    "nozzle_diameter": ["0.4"],
                    "printer_model": "U1",
                    "different_settings_to_system": ["old;override"]
                }
            },
            "slots": [
                {"color": "#112233", "type": "PETG", "profile": "Specific PETG"},
                {"color": "#778899", "type": "PLA"}
            ],
            "slot_count": 4,
            "supports": supports,
            "source_settings": {
                "support_type": "tree",
                "support_threshold_angle": "30",
                "support_on_build_plate_only": "1"
            }
        })
    }

    #[test]
    fn preserves_machine_keys_and_normalizes_filament_arrays() {
        let input = request(false);
        let output = compose_project_settings(&input).unwrap();
        assert_eq!(output["printer_model"], "U1");
        assert_eq!(output["nozzle_diameter"], json!(["0.4"]));
        assert_eq!(
            output["filament_colour"],
            json!(["#112233", "#778899", "#FFFFFF", "#FFFFFF"])
        );
        assert_eq!(
            output["filament_type"],
            json!(["PETG", "PLA", "PLA", "PLA"])
        );
        assert_eq!(
            output["filament_settings_id"],
            json!(["Specific PETG", "PLA Native", "PLA Native", "PLA Native"])
        );
        assert_eq!(
            output["filament_flow_ratio"],
            json!(["0.99", "0.99", "0.99", "0.99"])
        );
        assert_eq!(
            output["filament_density"],
            json!(["1.2", "1.3", "1.4", "1.5"])
        );
        assert_eq!(output["flush_volumes_matrix"].as_array().unwrap().len(), 16);
        assert_eq!(output["flush_volumes_matrix"][15], 1);
        assert_eq!(output["enable_support"], "0");
        assert!(output.get("different_settings_to_system").is_none());
        assert_eq!(
            input["profile"]["project_settings"]["filament_flow_ratio"],
            json!(["0.99"])
        );
    }

    #[test]
    fn supports_enabled_carries_only_named_source_overrides() {
        let output = compose_project_settings(&request(true)).unwrap();
        assert_eq!(output["enable_support"], "1");
        assert_eq!(output["support_type"], "tree");
        assert_eq!(output["support_threshold_angle"], "30");
        assert_eq!(output["support_on_build_plate_only"], "1");
        assert_eq!(
            output["different_settings_to_system"][0],
            "enable_support;support_type;support_threshold_angle;support_on_build_plate_only"
        );
        assert_eq!(
            output["different_settings_to_system"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        assert_eq!(output["different_settings_to_system"][5], "");
    }

    #[test]
    fn rejects_wrong_types_and_oversized_profile() {
        let mut invalid = request(false);
        invalid["slot_count"] = json!(65);
        assert!(compose_project_settings(&invalid).is_err());
        invalid["slot_count"] = json!(1);
        assert!(compose_project_settings(&invalid).is_err());
        invalid["slot_count"] = json!(4);
        invalid["slots"][0]["color"] = Value::Null;
        assert!(compose_project_settings(&invalid).is_err());
    }
}
