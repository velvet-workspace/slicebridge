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

pub fn experiment_path() -> Result<PathBuf, String> {
    Ok(config_root()?.join("SliceBridge/GPUI-Alpha/settings.json"))
}

fn stable_paths() -> Result<Vec<PathBuf>, String> {
    let root = config_root()?;
    Ok(vec![
        root.join("Velvet Factory/SliceBridge/settings.json"),
        root.join("Velvet Factory/3MF Bridge/settings.json"),
    ])
}

fn read_from_paths(alpha: &Path, fallbacks: &[PathBuf]) -> Result<Value, String> {
    // Never skip a corrupt experimental file or a corrupt stable source.
    // Reads do not normalize or rewrite the user's original bytes.
    for path in std::iter::once(alpha).chain(fallbacks.iter().map(PathBuf::as_path)) {
        if let Some(settings) = read_one(path)? {
            return Ok(settings);
        }
    }
    Ok(defaults())
}

pub fn current() -> Result<Value, String> {
    read_from_paths(&experiment_path()?, &stable_paths()?)
}

fn native_profile_verifier(
    target: &str,
    nozzle: &str,
    slicer: &str,
    preset: Option<(&str, &str)>,
) -> Result<(), String> {
    let verified_owner = match target {
        "u1" => "snapmaker-orca",
        "qidi" => "qidi-studio",
        _ => {
            return Err(format!(
                "installed profile validation for {target} has not been ported"
            ))
        }
    };
    if slicer != "auto" && slicer != verified_owner {
        return Err(format!(
            "slicer {slicer} has no verified Rust installed-profile resolver"
        ));
    }
    match preset {
        Some((material, name)) => {
            let installed = crate::oem::choices(target, nozzle, material)?;
            if !installed.iter().any(|item| item == name) {
                return Err(format!("{material} preset {name:?} is not a verified installed {target} {nozzle}mm preset"));
            }
            Ok(())
        }
        None => crate::oem::resolve(target, nozzle, &std::collections::BTreeMap::new()).map(|_| ()),
    }
}

fn validate_for_save(
    target: &str,
    preference: &Value,
    verify: impl Fn(&str, &str, &str, Option<(&str, &str)>) -> Result<(), String>,
) -> Result<Value, String> {
    let normalized = valid_setting(target, preference)?;
    let nozzle = normalized["nozzle"].as_str().expect("normalized nozzle");
    let slicer = normalized["slicer"].as_str().expect("normalized slicer");
    let overrides = normalized["filaments"]
        .as_object()
        .expect("normalized filaments");
    if nozzle == "auto"
        && overrides
            .values()
            .any(|name| name.as_str().is_some_and(|value| !value.is_empty()))
    {
        return Err("choose a fixed physical nozzle before saving a filament preset; automatic nozzle follows the source project".into());
    }
    let effective_nozzle = if nozzle == "auto" { "0.4" } else { nozzle };
    if slicer != "auto" {
        verify(target, effective_nozzle, slicer, None)?;
    }
    for (material, value) in overrides {
        let name = value.as_str().expect("validated filament name");
        if !name.is_empty() {
            verify(target, effective_nozzle, slicer, Some((material, name)))?;
        }
    }
    Ok(normalized)
}

fn save_in_context(
    alpha: &Path,
    fallbacks: &[PathBuf],
    target: &str,
    preference: &Value,
    verify: impl Fn(&str, &str, &str, Option<(&str, &str)>) -> Result<(), String>,
) -> Result<Value, String> {
    let normalized = validate_for_save(target, preference, verify)?;
    // A malformed baseline must never be silently reset or overwritten.
    let mut state = read_from_paths(alpha, fallbacks)?;
    state["printers"][target] = normalized;
    let serialized = serde_json::to_vec_pretty(&state)
        .map_err(|err| format!("encode printer settings: {err}"))?;
    if serialized.len() as u64 > MAX_BYTES {
        return Err("settings file exceeds 64 KiB".into());
    }
    let parent = alpha
        .parent()
        .ok_or("settings file has no parent directory")?;
    fs::create_dir_all(parent).map_err(|err| format!("create alpha settings directory: {err}"))?;
    let temporary = parent.join(format!(
        ".settings-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), String> {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|err| format!("create settings temporary file: {err}"))?;
        file.write_all(&serialized)
            .map_err(|err| format!("write settings: {err}"))?;
        file.write_all(b"\n")
            .map_err(|err| format!("finish settings: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("sync settings: {err}"))?;
        drop(file);
        replace_atomic(&temporary, alpha)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    // Re-read actual disk state, rather than claiming a save based on a write call alone.
    read_from_paths(alpha, fallbacks)
}

static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn replace_atomic(from: &Path, to: &Path) -> Result<(), String> {
    #[allow(unused_mut)] // Mutable for Windows transient sharing/lock retry.
    let mut result = fs::rename(from, to);
    #[cfg(target_os = "windows")]
    for _ in 0..20 {
        if ![Some(32), Some(33)]
            .contains(&result.as_ref().err().and_then(std::io::Error::raw_os_error))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        result = fs::rename(from, to);
    }
    result.map_err(|err| format!("replace settings atomically: {err}"))
}

pub fn save(target: &str, preference: &Value) -> Result<Value, String> {
    save_in_context(
        &experiment_path()?,
        &stable_paths()?,
        target,
        preference,
        native_profile_verifier,
    )
}

pub fn options(target: &str, nozzle: &str, slicer: &str, material: &str) -> Result<Value, String> {
    let (_, nozzles, _) = TARGETS
        .iter()
        .find(|item| item.0 == target)
        .ok_or_else(|| format!("invalid target printer {target}"))?;
    let current_nozzle = if nozzle.is_empty() || nozzle == "auto" {
        "0.4"
    } else {
        nozzle
    };
    if !nozzles.contains(&current_nozzle) {
        return Err(format!("unsupported physical nozzle {current_nozzle}"));
    }
    if !["PLA", "PETG", "ABS", "ASA", "TPU"].contains(&material) {
        return Err(format!("invalid material {material}"));
    }
    // The original frontend renders all known choices and disables absent ones.
    // Here installed=true is issued only by the existing Rust OEM validator.
    let candidates: &[(&str, &str)] = match target {
        "u1" => &[("snapmaker-orca", "Snapmaker Orca")],
        "qidi" => &[("qidi-studio", "QIDIStudio"), ("orca-slicer", "OrcaSlicer")],
        "c5" => &[("flash-studio", "Flash Studio Desktop")],
        "cc2" => &[("elegoo-slicer", "ElegooSlicer")],
        _ => &[],
    };
    let slicers: Vec<Value> = candidates.iter().map(|(id,name)| {
        json!({"id":id, "name":name, "installed":native_profile_verifier(target,current_nozzle,id,None).is_ok()})
    }).collect();
    let usable = slicer == "auto"
        || candidates.iter().any(|(id, _)| {
            *id == slicer && native_profile_verifier(target, current_nozzle, slicer, None).is_ok()
        });
    let filaments = if usable && nozzle != "auto" && !nozzle.is_empty() {
        crate::oem::choices(target, current_nozzle, material).unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(json!({"nozzles":nozzles,"slicers":slicers,"filaments":filaments}))
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
    fn writes_only_to_alpha_and_retains_all_other_preferences() {
        let root = std::env::temp_dir().join(format!(
            "sb-settings-alpha-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let alpha = root.join("alpha").join("settings.json");
        let stable = root.join("stable-settings.json");
        let stable_content = r#"{"version":1,"printers":{"qidi":{"nozzle":"0.6","slicer":"auto","filaments":{"PETG":""}}},"window":{"width":1400,"height":800},"developerMode":true}"#;
        fs::write(&stable, stable_content).unwrap();
        let updated = save_in_context(
            &alpha,
            std::slice::from_ref(&stable),
            "u1",
            &json!({"nozzle":"0.8","slicer":"auto","filaments":{}}),
            |_target, _nozzle, _slicer, _preset| Err("unexpected profile verification".into()),
        )
        .unwrap();
        assert_eq!(updated["printers"]["u1"]["nozzle"], "0.8");
        assert_eq!(updated["printers"]["qidi"]["nozzle"], "0.6");
        assert_eq!(updated["window"], json!({"width":1400,"height":800}));
        assert_eq!(updated["developerMode"], true);
        assert_eq!(fs::read_to_string(&stable).unwrap(), stable_content);
        let saved_raw = fs::read_to_string(&alpha).unwrap();
        assert!(saved_raw.contains("\"nozzle\": \"0.8\""));
        let next = save_in_context(
            &alpha,
            std::slice::from_ref(&stable),
            "cc2",
            &json!({"nozzle":"0.4","slicer":"auto","filaments":{}}),
            |_target, _nozzle, _slicer, _preset| Err("unexpected profile verification".into()),
        )
        .unwrap();
        assert_eq!(next["printers"]["u1"]["nozzle"], "0.8");
        assert_eq!(next["printers"]["cc2"]["nozzle"], "0.4");
        assert_eq!(fs::read_to_string(&stable).unwrap(), stable_content);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bad_preferences_cannot_write_or_reset_existing_files() {
        let root = std::env::temp_dir().join(format!(
            "sb-settings-bad-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let alpha = root.join("alpha.json");
        let stable = root.join("stable.json");
        fs::write(&stable, b"{future schema").unwrap();
        assert!(save_in_context(
            &alpha,
            &[stable.clone()],
            "u1",
            &json!({"nozzle":"0.4"}),
            |_, _, _, _| Ok(())
        )
        .is_err());
        assert!(!alpha.exists());
        assert_eq!(fs::read(&stable).unwrap(), b"{future schema");
        fs::write(&alpha, b"{broken alpha").unwrap();
        assert!(save_in_context(
            &alpha,
            &[stable.clone()],
            "u1",
            &json!({"nozzle":"0.6"}),
            |_, _, _, _| Ok(())
        )
        .is_err());
        assert_eq!(fs::read(&alpha).unwrap(), b"{broken alpha");
        assert_eq!(fs::read(&stable).unwrap(), b"{future schema");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn override_validation_requires_fixed_nozzle_and_installed_source() {
        assert!(validate_for_save(
            "u1",
            &json!({
                "nozzle":"auto","slicer":"auto","filaments":{"PLA":"Installed PLA"}
            }),
            |_, _, _, _| Ok(())
        )
        .is_err());
        assert!(validate_for_save(
            "u1",
            &json!({
                "nozzle":"0.6","slicer":"snapmaker-orca","filaments":{"PLA":"Installed PLA"}
            }),
            |_, _, _, _| Err("not installed".into())
        )
        .is_err());
        let calls = std::cell::RefCell::new(Vec::new());
        let selected = validate_for_save(
            "u1",
            &json!({
                "nozzle":"0.6","slicer":"snapmaker-orca","filaments":{"PLA":"Installed PLA"}
            }),
            |target, nozzle, slicer, preset| {
                calls.borrow_mut().push((
                    target.to_owned(),
                    nozzle.to_owned(),
                    slicer.to_owned(),
                    preset.map(|(a, b)| (a.to_owned(), b.to_owned())),
                ));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(selected["filaments"]["PLA"], "Installed PLA");
        assert_eq!(calls.borrow().len(), 2);
        assert_eq!(calls.borrow()[0].1, "0.6");
        assert_eq!(
            calls.borrow()[1].3,
            Some(("PLA".into(), "Installed PLA".into()))
        );
    }
    #[test]
    fn options_do_not_claim_uninstalled_or_unported_profile_support() {
        let c5 = options("c5", "0.4", "auto", "PLA").unwrap();
        assert!(c5["slicers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["installed"] == false));
        assert_eq!(c5["filaments"], json!([]));
        assert!(options("u1", "0.25", "auto", "PLA").is_err());
        assert!(options("qidi", "0.4", "auto", "RESIN").is_err());
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
