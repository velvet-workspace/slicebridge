//! Read-only port of SliceBridge native/preferences.go (settings load).
//! The stable Wails preference file is read as an optional migration source,
//! never modified by this implementation. Saving and profile-specific validation
//! will be enabled only when the complete installed-profile resolver is ported.
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 64 * 1024;
const TARGETS: &[(&str, &[&str], &[&str])] = &[
    (
        "u1",
        &["0.2", "0.4", "0.6", "0.8"],
        &["auto", "snapmaker-orca", "system"],
    ),
    (
        "c5",
        &["0.25", "0.4", "0.6", "0.8"],
        &["auto", "flash-studio", "system"],
    ),
    (
        "cc2",
        &["0.2", "0.4", "0.6", "0.8"],
        &["auto", "elegoo-slicer", "system"],
    ),
    (
        "qidi",
        &["0.2", "0.4", "0.6", "0.8"],
        &["auto", "qidi-studio", "orca-slicer", "system"],
    ),
];

pub fn defaults() -> Value {
    let mut printers = Map::new();
    for (target, _, _) in TARGETS {
        printers.insert(
            (*target).to_owned(),
            json!({"nozzle":"auto","slicer":"auto","filaments":{}}),
        );
    }
    json!({
        "version":1, "printers":printers,
        "window":{"width":1280,"height":900},
        "developerMode":false
    })
}

fn as_string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{name} must be text"))
}

fn valid_setting(target: &str, pref: &Value) -> Result<Value, String> {
    let Some((_, nozzles, slicers)) = TARGETS.iter().find(|row| row.0 == target) else {
        return Err(format!("unknown saved printer: {target}"));
    };
    if pref.is_null() {
        return Ok(json!({"nozzle":"auto","slicer":"auto","filaments":{}}));
    }
    let input = pref
        .as_object()
        .ok_or(format!("{target}: saved setting must be an object"))?;
    let nozzle = match input.get("nozzle") {
        Some(value) => as_string(value, "nozzle")?,
        None => "auto",
    };
    let slicer = match input.get("slicer") {
        Some(value) => as_string(value, "slicer")?,
        None => "auto",
    };
    let nozzle = if nozzle.is_empty() { "auto" } else { nozzle };
    let slicer = if slicer.is_empty() { "auto" } else { slicer };
    if nozzle != "auto" && !nozzles.contains(&nozzle) {
        return Err(format!("{target}: unsupported physical nozzle {nozzle}"));
    }
    if !slicers.contains(&slicer) {
        return Err(format!("{target}: invalid slicer selection {slicer}"));
    }
    // The existing Go backend exposes a separate profile-root override.
    // It is valid only if that trusted explicit override is configured.
    if slicer == "system"
        && std::env::var_os(format!("BRIDGE_SLICER_PROFILE_{}", target.to_uppercase())).is_none()
    {
        return Err(format!("{target}: system slicer override not configured"));
    }
    let filaments = input.get("filaments").filter(|value| !value.is_null());
    let mut selected = Map::new();
    if let Some(value) = filaments {
        let items = value
            .as_object()
            .ok_or(format!("{target}: filament settings must be an object"))?;
        if items.len() > 8 {
            return Err(format!("{target}: too many saved filament types"));
        }
        for (material, entry) in items {
            if !["PLA", "PETG", "ABS", "ASA", "TPU"].contains(&material.as_str()) {
                return Err(format!("{target}: invalid material type {material}"));
            }
            let name = as_string(entry, "filament preset")?;
            if name.len() > 200 || name.contains(['\0', '\r', '\n']) {
                return Err(format!("{target}: invalid saved {material} preset name"));
            }
            selected.insert(material.clone(), json!(name));
        }
    }
    Ok(json!({"nozzle":nozzle,"slicer":slicer,"filaments":selected}))
}

pub fn parse(raw: &[u8]) -> Result<Value, String> {
    if raw.len() as u64 > MAX_BYTES {
        return Err("settings file is too large".into());
    }
    let decoded: Value =
        serde_json::from_slice(raw).map_err(|e| format!("invalid settings JSON: {e}"))?;
    if decoded["version"] != 1 {
        return Err(format!(
            "unsupported settings schema: {}",
            decoded["version"]
        ));
    }
    let mut result = defaults();
    if let Some(printers) = decoded.get("printers").filter(|value| !value.is_null()) {
        let object = printers
            .as_object()
            .ok_or("printers settings must be an object")?;
        for (target, pref) in object {
            let normalized = valid_setting(target, pref)?;
            result["printers"][target] = normalized;
        }
    }
    if let Some(window) = decoded.get("window").filter(|value| !value.is_null()) {
        let window = window
            .as_object()
            .ok_or("window settings must be an object")?;
        for (dimension, min, max) in [("width", 720, 2000), ("height", 500, 1400)] {
            if let Some(input) = window.get(dimension) {
                let number = input
                    .as_i64()
                    .ok_or(format!("window {dimension} must be integer"))?;
                if number >= min && number <= max {
                    result["window"][dimension] = json!(number);
                }
            }
        }
    }
    if let Some(flag) = decoded
        .get("developerMode")
        .filter(|value| !value.is_null())
    {
        result["developerMode"] = json!(flag.as_bool().ok_or("developerMode must be boolean")?);
    }
    Ok(result)
}

pub fn read_one(path: &Path) -> Result<Option<Value>, String> {
    let metadata = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("read settings metadata: {err}")),
    };
    if !metadata.is_file() {
        return Err("settings are not a regular file".into());
    }
    if metadata.len() > MAX_BYTES {
        return Err("settings file is too large".into());
    }
    let bytes = fs::read(path).map_err(|err| format!("read settings file: {err}"))?;
    Ok(Some(parse(&bytes)?))
}

fn config_root() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.ok_or_else(|| "missing user config directory".into())
}

pub fn current() -> Result<Value, String> {
    let root = config_root()?;
    let candidate_paths = [
        // Alpha preference writes will be isolated here; no writes implemented.
        root.join("SliceBridge/GPUI-Alpha/settings.json"),
        // Reuse stable preferences read-only to match the installed product.
        root.join("Velvet Factory/SliceBridge/settings.json"),
        root.join("Velvet Factory/3MF Bridge/settings.json"),
    ];
    for path in candidate_paths {
        if let Some(settings) = read_one(&path)? {
            return Ok(settings);
        }
    }
    Ok(defaults())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_default_is_four_printers_and_1280x900() {
        let initial = defaults();
        assert_eq!(initial["printers"].as_object().unwrap().len(), 4);
        assert_eq!(initial["window"], json!({"width":1280,"height":900}));
        assert_eq!(initial["developerMode"], false);
    }
    #[test]
    fn saved_schema_retains_nozzle_slicer_filament_and_window_preferences() {
        let raw=br#"{"version":1,"printers":{"u1":{"nozzle":"0.6","slicer":"snapmaker-orca","filaments":{"PLA":"OEM PLA","TPU":"Generic TPU"}}},"window":{"width":1000,"height":700},"developerMode":true}"#;
        let current = parse(raw).unwrap();
        assert_eq!(current["printers"]["u1"]["nozzle"], "0.6");
        assert_eq!(current["printers"]["u1"]["slicer"], "snapmaker-orca");
        assert_eq!(current["printers"]["u1"]["filaments"]["PLA"], "OEM PLA");
        assert_eq!(current["window"]["width"], 1000);
        assert_eq!(current["developerMode"], true);
        assert_eq!(current["printers"]["qidi"]["nozzle"], "auto");
    }
    #[test]
    fn older_valid_preferences_get_only_missing_defaults() {
        let saved = parse(br#"{"version":1,"printers":{"c5":{"nozzle":"","slicer":"","filaments":null}},"window":{"width":500,"height":2000}}"#).unwrap();
        assert_eq!(
            saved["printers"]["c5"],
            json!({"nozzle":"auto","slicer":"auto","filaments":{}})
        );
        assert_eq!(saved["window"], json!({"width":1280,"height":900}));
    }
    #[test]
    fn corrupt_or_unknown_preferences_are_rejected_not_reset() {
        for raw in [
            r#"{"version":2,"printers":{}}"#,
            r#"{"version":1,"printers":{"unknown":{"nozzle":"auto"}}}"#,
            r#"{"version":1,"printers":{"u1":{"nozzle":"0.25"}}}"#,
            r#"{"version":1,"printers":{"qidi":{"filaments":{"resin":"Bad"}}}}"#,
            r#"{"version":1,"printers":{"cc2":{"slicer":"random-app"}}}"#,
            r#"{"version":1,"printers":{"u1":{"filaments":{"PLA":"line\nbreak"}}}}"#,
        ] {
            assert!(parse(raw.as_bytes()).is_err(), "unexpectedly parsed {raw}");
        }
    }
    #[test]
    fn files_are_never_modified_by_reading_or_invalid_schema_detection() {
        let path =
            std::env::temp_dir().join(format!("sb-settings-readonly-{}.json", std::process::id()));
        let invalid = b"{corrupt".to_vec();
        fs::write(&path, &invalid).unwrap();
        assert!(read_one(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), invalid);
        fs::remove_file(path).unwrap();
    }
}
