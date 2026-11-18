//! Read-only port of engine/bridge/nozzle.go::InspectNozzle.
//! The 3MF package is validated before its settings are inspected, and no
//! conversion, slicing, G-code or printer operation is possible on this path.
use serde_json::{json, Value};
use std::{fs::File, io::Read, path::Path};
use zip::{result::ZipError, ZipArchive};

const MAX_SETTINGS_BYTES: u64 = 8 << 20;
const VALID: [&str; 5] = ["0.2", "0.25", "0.4", "0.6", "0.8"];

fn normalize(value: &Value) -> Result<&'static str, String> {
    let input: f64 = match value {
        Value::String(raw) => raw
            .trim()
            .parse()
            .map_err(|_| format!("invalid nozzle diameter {raw:?}"))?,
        Value::Number(number) => number.as_f64().ok_or("invalid nozzle diameter")?,
        _ => return Err("invalid nozzle diameter type".into()),
    };
    if !input.is_finite() {
        return Err("invalid nozzle diameter".into());
    }
    for candidate in VALID {
        let expected = candidate.parse::<f64>().expect("known nozzle constant");
        if (input - expected).abs() < 0.000_001 {
            return Ok(candidate);
        }
    }
    Err(format!("unsupported nozzle diameter {input}"))
}

fn from_raw(raw: &Value) -> Result<Option<&'static str>, String> {
    if raw.is_null() {
        return Ok(None);
    }
    let entries = if let Value::Array(items) = raw {
        items.iter().collect::<Vec<_>>()
    } else {
        vec![raw]
    };
    if entries.len() > 16 {
        return Err("too many nozzle dimensions".into());
    }
    let mut selected = None;
    for item in entries {
        let next = normalize(item)?;
        if let Some(previous) = selected {
            if previous != next {
                return Err(format!(
                    "mixed nozzle diameters {previous} and {next} are not supported"
                ));
            }
        }
        selected = Some(next);
    }
    Ok(selected)
}

fn from_preset_name(name: &str) -> Option<&'static str> {
    // Equivalent to Go's nozzleInPreset:
    // (?i)(?:^|[^0-9])(0\.2[05]?|0\.4|0\.6|0\.8)\s*(?:mm\s*)?nozzle\b
    // Use an ASCII scan rather than a new regular-expression dependency.
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for index in 0..bytes.len() {
        if index > 0 && bytes[index - 1].is_ascii_digit() {
            continue;
        }
        for candidate in ["0.25", "0.20", "0.2", "0.4", "0.6", "0.8"] {
            if !bytes[index..].starts_with(candidate.as_bytes()) {
                continue;
            }
            let mut end = index + candidate.len();
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            if bytes[end..].starts_with(b"mm") {
                end += 2;
                while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                    end += 1;
                }
            }
            if !bytes[end..].starts_with(b"nozzle") {
                continue;
            }
            end += b"nozzle".len();
            if end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                continue;
            }
            return normalize(&json!(candidate)).ok();
        }
    }
    None
}

pub fn from_project_settings(settings: &Value) -> Result<Value, String> {
    if !settings.is_null() && !settings.is_object() {
        return Err("project settings JSON: expected object".into());
    }
    let mut explicit = from_raw(&settings["nozzle_diameter"])?;
    if explicit.is_none() {
        explicit = from_raw(&settings["nozzle_size"])?;
    }
    let mut from_preset = None;
    for field in ["printer_settings_id", "printer_model"] {
        let Some(value) = settings[field].as_str() else {
            continue;
        };
        let Some(next) = from_preset_name(value) else {
            continue;
        };
        if from_preset.is_some_and(|previous| previous != next) {
            return Err("conflicting nozzle sizes in project printer identifiers".into());
        }
        from_preset = Some(next);
    }
    if let (Some(explicit), Some(from_preset)) = (explicit, from_preset) {
        if explicit != from_preset {
            return Err(format!(
                "nozzle size in project settings ({explicit}) conflicts with printer preset ({from_preset})"
            ));
        }
    }
    if let Some(nozzle) = explicit {
        return Ok(json!({
            "source_nozzle":nozzle,
            "source_nozzle_source":"nozzle_diameter",
            "source_nozzle_assumed":false,
        }));
    }
    if let Some(nozzle) = from_preset {
        return Ok(json!({
            "source_nozzle":nozzle,
            "source_nozzle_source":"printer_settings_id",
            "source_nozzle_assumed":false,
        }));
    }
    Ok(json!({
        "source_nozzle":"0.4",
        "source_nozzle_source":"assumed_default",
        "source_nozzle_assumed":true,
    }))
}

/// Go's InspectNozzle verifies package geometry and ZIP layout before reading
/// a diameter. Never return a guessed nozzle for an invalid 3MF container.
pub fn inspect_source_nozzle(path: impl AsRef<Path>) -> Result<Value, String> {
    let path = path.as_ref();
    super::inspect_package(path).map_err(|err| format!("SOURCE_INVALID: {err}"))?;
    let file = File::open(path).map_err(|err| format!("SOURCE_INVALID: {err}"))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("SOURCE_INVALID: {err}"))?;
    let mut entry = match archive.by_name("Metadata/project_settings.config") {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => {
            return from_project_settings(&Value::Null)
                .map_err(|err| format!("SOURCE_NOZZLE_INVALID: {err}"))
        }
        Err(err) => return Err(format!("SOURCE_NOZZLE_INVALID: {err}")),
    };
    if entry.size() > MAX_SETTINGS_BYTES {
        return Err("SOURCE_NOZZLE_INVALID: oversized project settings".into());
    }
    let mut data = Vec::new();
    (&mut entry)
        .take(MAX_SETTINGS_BYTES)
        .read_to_end(&mut data)
        .map_err(|err| format!("SOURCE_NOZZLE_INVALID: {err}"))?;
    let parsed: Value =
        serde_json::from_slice(&data).map_err(|err| format!("SOURCE_NOZZLE_INVALID: {err}"))?;
    from_project_settings(&parsed).map_err(|err| format!("SOURCE_NOZZLE_INVALID: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn original_source_fixtures_match_reported_nozzle_and_assumption() {
        for (name, nozzle, assumed) in [
            ("bambu-one-color.3mf", "0.4", true),
            ("bambu-nozzle-0.2.3mf", "0.2", false),
            ("bambu-nozzle-0.4.3mf", "0.4", false),
            ("bambu-nozzle-0.6.3mf", "0.6", false),
            ("bambu-nozzle-0.8.3mf", "0.8", false),
            ("bambu-mixed-nozzle-0.2.3mf", "0.2", false),
            ("bambu-mixed-nozzle-0.6.3mf", "0.6", false),
            ("bambu-mixed-nozzle-0.8.3mf", "0.8", false),
        ] {
            let detected = inspect_source_nozzle(fixture(name)).unwrap();
            assert_eq!(detected["source_nozzle"], nozzle, "{name}");
            assert_eq!(detected["source_nozzle_assumed"], assumed, "{name}");
            assert_eq!(
                detected["source_nozzle_source"],
                if assumed {
                    "assumed_default"
                } else {
                    "nozzle_diameter"
                },
                "{name}",
            );
        }
    }

    #[test]
    fn original_identifier_fallbacks_and_decimal_equivalence() {
        assert_eq!(from_preset_name("Bambu A1 0.6 mm nozzle"), Some("0.6"));
        assert_eq!(from_preset_name("Creator 5 0.25 nozzle"), Some("0.25"));
        assert_eq!(from_preset_name("Bambu 0.3 nozzle"), None);
        assert_eq!(from_preset_name("Bambu Lab A1"), None);
        let from_preset = from_project_settings(&json!({
            "printer_settings_id":"Bambu X1 0.6 mm nozzle",
            "printer_model":"Bambu Lab X1"
        }))
        .unwrap();
        assert_eq!(from_preset["source_nozzle"], "0.6");
        assert_eq!(from_preset["source_nozzle_source"], "printer_settings_id");
        assert_eq!(from_raw(&json!([0.2000001, "0.2"])).unwrap(), Some("0.2"));
        assert!(
            from_raw(&json!(["0.6", "0.8"]))
                .unwrap_err()
                .contains("mixed nozzle diameters")
        );
    }

    #[test]
    fn malformed_mixed_or_conflicting_nozzle_is_rejected() {
        for value in [
            json!({"nozzle_diameter":["0.6","0.8"]}),
            json!({"nozzle_diameter":["0.6"],"printer_settings_id":"A1 0.4 nozzle"}),
            json!({"printer_settings_id":"A1 0.4 nozzle","printer_model":"A1 0.6 nozzle"}),
            json!({"nozzle_size":"0.3"}),
            json!({"nozzle_diameter":vec!["0.4";17]}),
            json!({"nozzle_diameter":false}),
        ] {
            assert!(from_project_settings(&value).is_err(), "{value}");
        }
        assert!(from_project_settings(&json!(["invalid"])).is_err());
    }

    #[test]
    fn missing_fields_are_explicitly_reported_as_assumed() {
        assert_eq!(
            from_project_settings(&Value::Null).unwrap()["source_nozzle_assumed"],
            true
        );
        assert_eq!(
            from_project_settings(&json!({})).unwrap()["source_nozzle"],
            "0.4"
        );
        assert!(inspect_source_nozzle(fixture("missing.3mf")).is_err());
    }
}
