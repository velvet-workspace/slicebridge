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
    // Go json.Unmarshal leaves these string fields empty for JSON null, and
    // loadSettings then normalizes "" to "auto", so null is accepted, not rejected.
    let nozzle = match input.get("nozzle") {
        Some(value) if value.is_null() => "",
        Some(value) => as_string(value, "nozzle")?,
        None => "auto",
    };
    let slicer = match input.get("slicer") {
        Some(value) if value.is_null() => "",
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
            // Go's json.Unmarshal decodes a null map[string]string entry to an
            // empty string. Preserve it; invalid non-null types still fail.
            let name = if entry.is_null() {
                ""
            } else {
                as_string(entry, "filament preset")?
            };
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

// The original Wails app asks before recovering malformed preferences. The GPUI
// alpha always writes only its own file: even when a stable Wails file was corrupt,
// preserve its original bytes and keep the installed app untouched.
fn recover_invalid_in_context(alpha: &Path, fallbacks: &[PathBuf]) -> Result<Value, String> {
    if let Ok(valid) = read_from_paths(alpha, fallbacks) {
        return Ok(json!({"settings":valid,"backupPath":null}));
    }

    let mut selected: Option<&Path> = None;
    for path in std::iter::once(alpha).chain(fallbacks.iter().map(PathBuf::as_path)) {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                selected = Some(path);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect original settings: {error}")),
        }
    }
    let source = selected.ok_or("settings could not be read; no recoverable file was found")?;
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("inspect original settings: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("settings recovery requires a regular file, not a symlink".into());
    }
    // A maliciously large file should not be copied into the alpha profile.
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("settings backup is unexpectedly large".into());
    }
    let original = fs::read(source).map_err(|error| format!("read original settings: {error}"))?;
    if parse(&original).is_ok() {
        return Err("settings changed during recovery; reopen settings".into());
    }

    let parent = alpha.parent().ok_or("no alpha settings directory")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create settings backup directory: {error}"))?;
    let backup = parent.join(format!(
        "settings.json.invalid-{}-{}.bak",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&backup)
        .map_err(|error| format!("create settings backup: {error}"))?;
    let backup_result = (|| -> Result<(), String> {
        use std::io::Write;
        file.write_all(&original)
            .map_err(|error| format!("copy settings backup: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync settings backup: {error}"))?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = backup_result {
        let _ = fs::remove_file(&backup);
        return Err(error);
    }
    // An external writer may have repaired the file while the backup was made.
    // Refuse to overwrite its new bytes.
    if fs::read(source).map_err(|error| format!("re-read original settings: {error}"))? != original
    {
        return Err("settings changed during recovery; reopen settings".into());
    }
    let restored = persist_settings_in_context(alpha, &[], &defaults())?;
    Ok(json!({
        "settings": restored,
        "backupPath": backup.to_string_lossy()
    }))
}

pub fn recover_invalid() -> Result<Value, String> {
    recover_invalid_in_context(&experiment_path()?, &stable_paths()?)
}

fn native_profile_verifier(
    target: &str,
    nozzle: &str,
    slicer: &str,
    preset: Option<(&str, &str)>,
) -> Result<(), String> {
    let mut selected = std::collections::BTreeMap::new();
    if let Some((material, name)) = preset {
        selected.insert(material.to_owned(), name.to_owned());
    }
    // Validate through the selected installed vendor/fork, never through a
    // different source chosen by auto. This also covers Creator5, CC2 and
    // genuine OrcaSlicer fallback without any generated profiles.
    crate::oem::resolve_configured(target, nozzle, slicer, &selected).map(|_| ())
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
    persist_settings_in_context(alpha, fallbacks, &state)
}

// Shared atomic, readback-verified write for printer preferences and Developer
// Mode. Both operations target only GPUI-Alpha/settings.json, never stable Go.
fn persist_settings_in_context(
    alpha: &Path,
    fallbacks: &[PathBuf],
    state: &Value,
) -> Result<Value, String> {
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

fn set_developer_mode_in_context(
    alpha: &Path,
    fallbacks: &[PathBuf],
    enabled: bool,
) -> Result<Value, String> {
    // Mirrors native/preferences.go::SetDeveloperMode: read/validate first,
    // refuse corrupt settings, persist the boolean and verify by readback.
    let mut state = read_from_paths(alpha, fallbacks)?;
    state["developerMode"] = json!(enabled);
    let persisted = persist_settings_in_context(alpha, fallbacks, &state)?;
    if persisted["developerMode"] != enabled {
        return Err("Developer Mode persistence was not verified".into());
    }
    Ok(persisted)
}

pub fn set_developer_mode(enabled: bool) -> Result<Value, String> {
    set_developer_mode_in_context(&experiment_path()?, &stable_paths()?, enabled)
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

// Translate the original fixed physical-nozzle and installed-filament preferences
// into an OEM profile request. Explicit agent nozzle choices remain explicit.
/// Port of engine/bridge/targets.go::targetNozzleForSource.
pub fn target_nozzle_for_source(target: &str, source: &str) -> Result<&'static str, String> {
    let (_, nozzles, _) = TARGETS
        .iter()
        .find(|row| row.0 == target)
        .ok_or_else(|| format!("unknown selected printer {target}"))?;
    for nozzle in *nozzles {
        if *nozzle == source {
            return Ok(nozzle);
        }
    }
    if source == "0.2" && nozzles.contains(&"0.25") {
        return Ok("0.25");
    }
    if source == "0.25" && nozzles.contains(&"0.2") {
        return Ok("0.2");
    }
    Err(format!(
        "no matching {target} target nozzle for source {source} mm"
    ))
}

/// A fixed saved physical nozzle overrides the source like original Go.
/// In automatic mode, the source project determines the target hardware
/// variant (including the 0.2/0.25 manufacturer-specific mapping).
pub fn profile_choice_for_source(
    settings: &Value,
    target: &str,
    requested: &str,
    source: &str,
) -> Result<(String, std::collections::BTreeMap<String, String>), String> {
    if requested != "auto" {
        return profile_choice(settings, target, requested);
    }
    let saved = settings["printers"][target]["nozzle"]
        .as_str()
        .unwrap_or("auto");
    if saved != "auto" {
        return profile_choice(settings, target, "auto");
    }
    let mapped = target_nozzle_for_source(target, source)?;
    profile_choice(settings, target, mapped)
}

pub fn profile_choice(
    settings: &Value,
    target: &str,
    requested_nozzle: &str,
) -> Result<(String, std::collections::BTreeMap<String, String>), String> {
    let (_, valid_nozzles, valid_slicers) = TARGETS
        .iter()
        .find(|entry| entry.0 == target)
        .ok_or("unknown selected target")?;
    let pref = &settings["printers"][target];
    let saved_nozzle = pref["nozzle"].as_str().unwrap_or("auto");
    let slicer = pref["slicer"].as_str().unwrap_or("auto");
    if !valid_slicers.contains(&slicer) {
        return Err(format!("saved slicer {slicer} is not valid for {target}"));
    }
    if slicer == "system"
        && std::env::var_os(format!(
            "BRIDGE_SLICER_PROFILE_{}",
            target.to_ascii_uppercase()
        ))
        .is_none()
    {
        return Err(format!(
            "{target}: system slicer override is not configured"
        ));
    }
    if requested_nozzle != "auto" && saved_nozzle != "auto" && requested_nozzle != saved_nozzle {
        return Err(format!("requested {requested_nozzle}mm nozzle differs from saved physical {saved_nozzle}mm nozzle"));
    }
    let effective = if saved_nozzle != "auto" {
        saved_nozzle
    } else if requested_nozzle != "auto" {
        requested_nozzle
    } else {
        "0.4"
    };
    if !valid_nozzles.contains(&effective) {
        return Err(format!(
            "{target} does not have a verified {effective}mm nozzle"
        ));
    }
    let mut materials = std::collections::BTreeMap::new();
    if let Some(overrides) = pref["filaments"].as_object() {
        for (material, preset) in overrides {
            if let Some(name) = preset.as_str().filter(|name| !name.is_empty()) {
                materials.insert(material.clone(), name.to_owned());
            }
        }
    }
    if saved_nozzle == "auto" && !materials.is_empty() {
        return Err("saved filament presets require a fixed physical nozzle".into());
    }
    Ok((effective.to_owned(), materials))
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
    // Source-locked installed slicer identities, including a trusted
    // BRIDGE_SLICER_PROFILE_* "system" choice when it replaces app discovery.
    // Never advertise a different app while an explicit override is active.
    let slicers = crate::oem::slicer_options(target, current_nozzle)?;
    let usable = slicer == "auto"
        || slicers
            .iter()
            .any(|row| row["id"] == slicer && row["installed"] == true);
    let filaments = if usable && nozzle != "auto" && !nozzle.is_empty() {
        crate::oem::choices_configured(target, current_nozzle, slicer, material).unwrap_or_default()
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
    fn json_null_nozzle_and_slicer_read_as_auto_like_go_loader() {
        // Go json.Unmarshal keeps "" for JSON null in string fields, and
        // loadSettings (native/preferences.go) maps "" to "auto" without error.
        let saved = parse(
            br#"{"version":1,"printers":{"u1":{"nozzle":null,"slicer":null,"filaments":{}}}}"#,
        )
        .unwrap();
        assert_eq!(
            saved["printers"]["u1"],
            json!({"nozzle":"auto","slicer":"auto","filaments":{}})
        );
        // A non-null, non-string nozzle still fails, as Go's unmarshal type error does.
        assert!(parse(br#"{"version":1,"printers":{"u1":{"nozzle":0.4}}}"#).is_err());
    }
    #[test]
    fn json_null_filament_name_matches_go_map_string_loader() {
        let saved =
            parse(br#"{"version":1,"printers":{"u1":{"filaments":{"PLA":null}}}}"#).unwrap();
        assert_eq!(saved["printers"]["u1"]["filaments"]["PLA"], "");
        for raw in [
            r#"{"version":1,"printers":{"u1":{"filaments":{"PLA":0}}}}"#,
            r#"{"version":1,"printers":{"u1":{"filaments":{"PLA":false}}}}"#,
            r#"{"version":1,"printers":{"u1":{"filaments":{"PLA":[]}}}}"#,
            r#"{"version":1,"printers":{"u1":{"filaments":{"PLA":{}}}}}"#,
        ] {
            assert!(parse(raw.as_bytes()).is_err(), "unexpected {raw}");
        }
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
    fn original_source_to_target_nozzle_mapping_is_preserved() {
        for (source, target, expected) in [
            ("0.2", "u1", "0.2"),
            ("0.2", "c5", "0.25"),
            ("0.2", "cc2", "0.2"),
            ("0.2", "qidi", "0.2"),
            ("0.25", "u1", "0.2"),
            ("0.25", "cc2", "0.2"),
            ("0.25", "c5", "0.25"),
            ("0.25", "qidi", "0.2"),
            ("0.4", "u1", "0.4"),
            ("0.6", "qidi", "0.6"),
            ("0.8", "c5", "0.8"),
        ] {
            assert_eq!(target_nozzle_for_source(target, source).unwrap(), expected);
        }
        assert!(target_nozzle_for_source("u1", "0.3").is_err());
        assert!(target_nozzle_for_source("unknown", "0.4").is_err());
    }

    #[test]
    fn auto_uses_source_unless_physical_nozzle_is_saved() {
        let mut saved = defaults();
        assert_eq!(
            profile_choice_for_source(&saved, "u1", "auto", "0.8")
                .unwrap()
                .0,
            "0.8"
        );
        assert_eq!(
            profile_choice_for_source(&saved, "c5", "auto", "0.2")
                .unwrap()
                .0,
            "0.25"
        );
        assert_eq!(
            profile_choice_for_source(&saved, "qidi", "auto", "0.25")
                .unwrap()
                .0,
            "0.2"
        );
        saved["printers"]["u1"]["nozzle"] = json!("0.6");
        assert_eq!(
            profile_choice_for_source(&saved, "u1", "auto", "0.2")
                .unwrap()
                .0,
            "0.6"
        );
        assert!(profile_choice_for_source(&saved, "u1", "0.4", "0.2").is_err());
    }

    #[test]
    fn profile_selection_applies_fixed_nozzle_and_filament_and_fails_closed() {
        let mut saved = defaults();
        saved["printers"]["u1"] = json!({
            "nozzle":"0.6","slicer":"snapmaker-orca",
            "filaments":{"PLA":"Compatible custom PLA","PETG":""}
        });
        let (nozzle, overrides) = profile_choice(&saved, "u1", "auto").unwrap();
        assert_eq!(nozzle, "0.6");
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides.get("PLA").unwrap(), "Compatible custom PLA");
        assert!(profile_choice(&saved, "u1", "0.4").is_err());
        assert_eq!(profile_choice(&saved, "u1", "0.6").unwrap().0, "0.6");
        saved["printers"]["u1"]["slicer"] = json!("wrong-slicer");
        assert!(profile_choice(&saved, "u1", "auto").is_err());
        saved["printers"]["u1"] = json!({
            "nozzle":"auto","slicer":"auto","filaments":{"PLA":"Unpinned"}
        });
        assert!(profile_choice(&saved, "u1", "auto").is_err());
        saved["printers"]["u1"]["filaments"] = json!({});
        assert_eq!(profile_choice(&saved, "u1", "auto").unwrap().0, "0.4");
        assert_eq!(profile_choice(&saved, "u1", "0.8").unwrap().0, "0.8");
        assert!(profile_choice(&saved, "unknown", "auto").is_err());
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
    fn options_report_installed_profile_readiness_without_inventing_slicers() {
        let c5 = options("c5", "0.4", "auto", "PLA").unwrap();
        let slicers = c5["slicers"].as_array().unwrap();
        assert!(slicers
            .iter()
            .all(|row| { row["id"] == "flash-studio" || row["id"] == "system" }));
        for row in slicers {
            let id = row["id"].as_str().unwrap();
            assert_eq!(
                row["installed"] == true,
                native_profile_verifier("c5", "0.4", id, None).is_ok()
            );
        }
        if !slicers.iter().any(|row| row["installed"] == true) {
            assert_eq!(c5["filaments"], json!([]));
        }
        assert!(options("u1", "0.25", "auto", "PLA").is_err());
        assert!(options("qidi", "0.4", "auto", "RESIN").is_err());
    }

    #[test]
    fn explicit_saved_slicer_selection_keeps_valid_source_nozzle_mapping() {
        let mut saved = defaults();
        saved["printers"]["qidi"]["slicer"] = json!("orca-slicer");
        saved["printers"]["qidi"]["nozzle"] = json!("auto");
        let chosen = profile_choice_for_source(&saved, "qidi", "auto", "0.25").unwrap();
        assert_eq!(chosen.0, "0.2");
        assert_eq!(saved["printers"]["qidi"]["slicer"], "orca-slicer");
        saved["printers"]["c5"]["slicer"] = json!("flash-studio");
        assert_eq!(
            profile_choice_for_source(&saved, "c5", "auto", "0.2")
                .unwrap()
                .0,
            "0.25"
        );
        saved["printers"]["qidi"]["slicer"] = json!("invented");
        assert!(profile_choice_for_source(&saved, "qidi", "auto", "0.4").is_err());
    }

    #[test]
    fn developer_mode_writes_only_alpha_and_reads_back_without_resetting_preferences() {
        let root = std::env::temp_dir().join(format!("sb-dev-mode-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let stable = root.join("stable/settings.json");
        let alpha = root.join("alpha/settings.json");
        fs::create_dir_all(stable.parent().unwrap()).unwrap();
        let stable_raw = br#"{"version":1,"printers":{"qidi":{"nozzle":"0.6","slicer":"auto","filaments":{}}},"window":{"width":1500,"height":960},"developerMode":false}"#;
        fs::write(&stable, stable_raw).unwrap();
        let fallbacks = [stable.clone()];

        let enabled = set_developer_mode_in_context(&alpha, &fallbacks, true).unwrap();
        assert_eq!(enabled["developerMode"], true);
        assert_eq!(enabled["printers"]["qidi"]["nozzle"], "0.6");
        assert_eq!(enabled["window"], json!({"width":1500,"height":960}));
        assert_eq!(fs::read(&stable).unwrap(), stable_raw);
        assert!(alpha.is_file());
        assert_eq!(read_one(&alpha).unwrap().unwrap()["developerMode"], true);

        let disabled = set_developer_mode_in_context(&alpha, &fallbacks, false).unwrap();
        assert_eq!(disabled["developerMode"], false);
        assert_eq!(fs::read(&stable).unwrap(), stable_raw);
        assert_eq!(read_one(&alpha).unwrap().unwrap()["developerMode"], false);

        let invalid = b"{damaged alpha settings".to_vec();
        fs::write(&alpha, &invalid).unwrap();
        assert!(set_developer_mode_in_context(&alpha, &fallbacks, true).is_err());
        assert_eq!(fs::read(&alpha).unwrap(), invalid);
        assert_eq!(fs::read(&stable).unwrap(), stable_raw);

        fs::remove_file(&alpha).unwrap();
        let corrupt_stable = b"{damaged stable settings".to_vec();
        fs::write(&stable, &corrupt_stable).unwrap();
        assert!(set_developer_mode_in_context(&alpha, &fallbacks, true).is_err());
        assert!(!alpha.exists());
        assert_eq!(fs::read(&stable).unwrap(), corrupt_stable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_recovery_backs_up_corrupt_alpha_before_restoring_defaults() {
        let root = std::env::temp_dir().join(format!(
            "sb-confirmed-recovery-alpha-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let alpha = root.join("GPUI-Alpha").join("settings.json");
        fs::create_dir_all(alpha.parent().unwrap()).unwrap();
        let original = br#"{"version":99,"printers":{"future":"keep exact bytes"}}"#;
        fs::write(&alpha, original).unwrap();
        assert!(read_from_paths(&alpha, &[]).is_err());
        assert_eq!(fs::read(&alpha).unwrap(), original);

        let reply = recover_invalid_in_context(&alpha, &[]).unwrap();
        let backup = PathBuf::from(reply["backupPath"].as_str().unwrap());
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(reply["settings"], defaults());
        assert_eq!(read_from_paths(&alpha, &[]).unwrap(), defaults());
        assert_ne!(backup, alpha);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmed_recovery_never_modifies_corrupt_stable_wails_file() {
        let root = std::env::temp_dir().join(format!(
            "sb-confirmed-recovery-stable-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let alpha = root.join("GPUI-Alpha").join("settings.json");
        let stable = root.join("stable-settings.json");
        let original = b"{invalid stable settings!";
        fs::write(&stable, original).unwrap();
        assert!(!alpha.exists());
        assert!(read_from_paths(&alpha, std::slice::from_ref(&stable)).is_err());

        let reply = recover_invalid_in_context(&alpha, std::slice::from_ref(&stable)).unwrap();
        let backup = PathBuf::from(reply["backupPath"].as_str().unwrap());
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(fs::read(&stable).unwrap(), original);
        assert_eq!(reply["settings"], defaults());
        assert_eq!(
            read_from_paths(&alpha, std::slice::from_ref(&stable)).unwrap(),
            defaults()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn valid_settings_never_trigger_recovery_or_alpha_write() {
        let root = std::env::temp_dir().join(format!(
            "sb-confirmed-recovery-valid-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let alpha = root.join("GPUI-Alpha").join("settings.json");
        let stable = root.join("stable-settings.json");
        let original = br#"{"version":1,"printers":{"u1":{"nozzle":"0.6"}}}"#;
        fs::write(&stable, original).unwrap();

        let reply = recover_invalid_in_context(&alpha, std::slice::from_ref(&stable)).unwrap();
        assert!(reply["backupPath"].is_null());
        assert_eq!(reply["settings"]["printers"]["u1"]["nozzle"], "0.6");
        assert_eq!(fs::read(&stable).unwrap(), original);
        assert!(!alpha.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_preferences_cannot_be_recovered_or_overwritten() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "sb-confirmed-recovery-link-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("protected.json");
        fs::write(&target, b"{invalid original").unwrap();
        let alpha = root.join("settings.json");
        symlink(&target, &alpha).unwrap();
        assert!(recover_invalid_in_context(&alpha, &[]).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"{invalid original");
        assert!(fs::symlink_metadata(&alpha)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(root).unwrap();
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
