//! SliceBridge stable "My Printers" catalogue and selection parity.
//!
//! Mirrors native/printer_catalog.go. Inventory membership does not imply a
//! verified conversion target. The isolated GPUI alpha path cannot overwrite
//! the existing Wails installation's my-printers.json.
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};
static TEMP_INDEX: AtomicU64 = AtomicU64::new(0);
const MAX_PICKER_BYTES: usize = 1024 * 1024;
const MAX_SAVED_BYTES: u64 = 32 * 1024;
const MAX_MY_PRINTERS: usize = 32;
const LEGACY: [(&str, &str); 4] = [
    ("u1", "snapmaker-u1"),
    ("c5", "flashforge-creator-5"),
    ("cc2", "elegoo-centauri-carbon-2"),
    ("qidi", "qidi-x-plus-5"),
];

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn installed_shape(id: &str) -> bool {
    // Same pattern as native/presets/{orca,prusa}.go:
    // ^(orca|prusa)-[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{10}$
    if !(id.starts_with("orca-") || id.starts_with("prusa-")) {
        return false;
    }
    let Some((prefix, fingerprint)) = id.rsplit_once('-') else {
        return false;
    };
    prefix.len() > 5
        && fingerprint.len() == 10
        && fingerprint
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && valid_id(id)
}

fn source_catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../native/frontend/dist/printer-catalog.json"
        )))
        .expect("the committed stable catalogue must be valid JSON")
    })
}

pub fn catalog_snapshot() -> Result<Value, String> {
    let raw = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../native/frontend/dist/printer-catalog.json"
    ));
    if raw.len() > MAX_PICKER_BYTES {
        return Err("embedded catalogue exceeds limit".into());
    }
    let catalog = source_catalog();
    let models = catalog["models"]
        .as_array()
        .ok_or("missing offline catalogue models")?;
    if catalog["schemaVersion"] != 1
        || catalog["kind"] != "slicebridge-offline-printer-picker"
        || catalog["modelCount"] != 603
        || catalog["manufacturers"] != 84
        || catalog["legacyAdapterCount"] != 4
        || catalog["thirdPartyProfilePayloadsIncluded"] != false
        || models.len() != 603
    {
        return Err("embedded catalogue schema or source count mismatch".into());
    }
    let mut ids = HashSet::new();
    let mut makers = HashSet::new();
    let mut adapters = BTreeSet::new();
    for model in models {
        let id = model["id"].as_str().ok_or("missing catalogue ID")?;
        let maker = model["manufacturer"]
            .as_str()
            .ok_or("missing manufacturer")?;
        let title = model["model"].as_str().ok_or("missing model")?;
        if !valid_id(id)
            || !ids.insert(id)
            || maker.trim().is_empty()
            || maker.len() > 100
            || title.trim().is_empty()
            || title.len() > 160
            || !["index_only", "structural_verified"]
                .contains(&model["structuralStatus"].as_str().unwrap_or(""))
            || !model["imagePath"].is_null()
            || !model["logoPath"].is_null()
        {
            return Err("invalid or duplicate catalogue printer".into());
        }
        makers.insert(maker);
        if let Some(adapter) = model["legacyTarget"].as_str() {
            if !LEGACY.contains(&(adapter, id)) || !adapters.insert(adapter) {
                return Err("catalogue claims an unsupported conversion adapter".into());
            }
        }
    }
    if makers.len() != 84 || adapters.len() != LEGACY.len() {
        return Err("catalogue maker/adapter totals do not match".into());
    }
    Ok(catalog.clone())
}

fn selectable(id: &str, catalog: &Value) -> bool {
    catalog["models"].as_array().is_some_and(|models| {
        models.iter().any(|item| {
            item["id"] == id
                && item["manufacturer"]
                    .as_str()
                    .is_some_and(|maker| !maker.eq_ignore_ascii_case("Bambu Lab"))
        })
    })
}

fn validate_new(
    ids: &[String],
    catalog: &Value,
    available: &HashSet<String>,
) -> Result<(), String> {
    if ids.len() > MAX_MY_PRINTERS {
        return Err(format!(
            "at most {MAX_MY_PRINTERS} printers can be selected"
        ));
    }
    let mut seen = HashSet::new();
    for id in ids {
        if !valid_id(id) || !seen.insert(id) {
            return Err("invalid or duplicate printer selection".into());
        }
        if id == "prusa-project" || selectable(id, catalog) {
            continue;
        }
        if installed_shape(id) {
            if !available.contains(id) {
                return Err("installed printer profile is not ready".into());
            }
            continue;
        }
        return Err("printer is not in the offline catalogue".into());
    }
    Ok(())
}

fn validated_saved(
    ids: &[String],
    catalog: &Value,
    installed: &HashSet<String>,
) -> Result<Vec<String>, String> {
    if ids.len() > MAX_MY_PRINTERS {
        return Err(format!(
            "saved printer selections invalid: at most {MAX_MY_PRINTERS} printers"
        ));
    }
    let mut seen = HashSet::new();
    let mut visible = Vec::new();
    for id in ids {
        if !valid_id(id) || !seen.insert(id) {
            return Err(
                "saved printer selections invalid: invalid or duplicate printer selection".into(),
            );
        }
        if id == "prusa-project"
            || selectable(id, catalog)
            || (installed_shape(id) && installed.contains(id))
        {
            visible.push(id.clone());
        }
        // Unknown and currently absent targets do not become conversion options,
        // and they do not disappear from the existing saved file on read.
    }
    Ok(visible)
}

fn load(
    path: &Path,
    catalog: &Value,
    available: &HashSet<String>,
) -> Result<(Value, Vec<String>), String> {
    let bytes = match fs::metadata(path) {
        Ok(metadata) => {
            if metadata.len() > MAX_SAVED_BYTES {
                return Err("saved printer selections exceed limit".into());
            }
            fs::read(path).map_err(|e| format!("read printer preferences: {e}"))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok((
                json!({"version":1, "selectedIds":[], "onboardingPending":true}),
                Vec::new(),
            ));
        }
        Err(err) => return Err(format!("read printer preferences: {err}")),
    };
    if bytes.len() as u64 > MAX_SAVED_BYTES {
        return Err("saved printer selections exceed limit".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("corrupt saved printer selections: {e}"))?;
    let Some(items) = value["selectedIds"].as_array() else {
        return Err("unrecognized saved printer selection version".into());
    };
    if value["version"] != 1 {
        return Err("unrecognized saved printer selection version".into());
    }
    let saved: Vec<String> = items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or("invalid saved printer ID".to_owned())
        })
        .collect::<Result<_, _>>()?;
    let visible = validated_saved(&saved, catalog, available)?;
    Ok((
        json!({"version":1, "selectedIds":visible, "onboardingPending":false}),
        saved,
    ))
}

pub fn read(path: &Path, available: &HashSet<String>) -> Result<Value, String> {
    let catalog = catalog_snapshot()?;
    Ok(load(path, &catalog, available)?.0)
}

pub fn save(
    path: &Path,
    requested: &[String],
    available: &HashSet<String>,
) -> Result<Value, String> {
    let catalog = catalog_snapshot()?;
    validate_new(requested, &catalog, available)?;
    // Bad existing state may not be silently reset.
    let (_, saved) = load(path, &catalog, available)?;
    let mut result = requested.to_vec();
    let selected: HashSet<&str> = requested.iter().map(String::as_str).collect();
    // The existing Go backend retains absent dynamic Orca and Prusa printer
    // profiles that were not visible for deselection in the dialog.
    for id in saved {
        if installed_shape(&id) && !available.contains(&id) && !selected.contains(id.as_str()) {
            result.push(id);
        }
    }
    if result.len() > MAX_MY_PRINTERS {
        return Err(format!(
            "at most {MAX_MY_PRINTERS} printers can be selected"
        ));
    }
    let parent = path
        .parent()
        .ok_or("missing printer settings parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("create printer settings directory: {e}"))?;
    let json = serde_json::to_vec_pretty(&json!({"version":1, "selectedIds":result}))
        .map_err(|e| format!("encode printer selection: {e}"))?;
    let temp = parent.join(format!(
        ".my-printers-{}-{}.tmp",
        std::process::id(),
        TEMP_INDEX.fetch_add(1, Ordering::Relaxed)
    ));
    let write_result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file: File = options
            .open(&temp)
            .map_err(|e| format!("temporary preferences file: {e}"))?;
        file.write_all(&json)
            .map_err(|e| format!("write printer preferences: {e}"))?;
        file.write_all(b"\n")
            .map_err(|e| format!("finish printer preferences: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync printer preferences: {e}"))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| format!("replace printer preferences: {e}"))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temp);
    }
    write_result?;
    Ok(load(path, &catalog, available)?.0)
}

pub fn experimental_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.map(|base| base.join("SliceBridge/GPUI-Alpha/my-printers.json"))
        .ok_or_else(|| "user config directory is not available".to_string())
}

pub fn available_installed_ids() -> HashSet<String> {
    // Membership is based on fully resolved installed user presets, never on
    // an app folder or a catalogue match. The batch engine remains separately
    // gated: no unported dynamic Orca/Prusa ID is a conversion-ready target.
    crate::prusa::installed_prusa_target_ids()
        .into_iter()
        .chain(crate::orca_profiles::installed_ids())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "slicebridge-printers-{}-{}-{}",
                std::process::id(),
                TEMP_INDEX.fetch_add(1, Ordering::Relaxed),
                name
            ))
            .join("my-printers.json")
    }
    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn exact_original_catalogue_and_non_bambu_availability() {
        let catalog = catalog_snapshot().unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 603);
        assert_eq!(catalog["manufacturers"], 84);
        assert_eq!(catalog["legacyAdapterCount"], 4);
        assert_eq!(
            catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["manufacturer"] == "Bambu Lab")
                .count(),
            14
        );
        assert!(selectable("snapmaker-u1", &catalog));
        assert!(!selectable("bambu-lab-a1", &catalog));
    }
    #[test]
    fn saves_known_inventory_without_claiming_conversion_support() {
        let path = temp("save");
        let initial = read(&path, &none()).unwrap();
        assert_eq!(initial["selectedIds"], json!([]));
        assert_eq!(initial["onboardingPending"], true);
        let selected = vec![
            "snapmaker-u1".into(),
            "flashforge-creator-5".into(),
            "qidi-x-plus-5".into(),
        ];
        let saved = save(&path, &selected, &none()).unwrap();
        assert_eq!(saved["selectedIds"], json!(selected));
        assert_eq!(saved["onboardingPending"], false);
        assert_eq!(
            read(&path, &none()).unwrap()["selectedIds"],
            json!(selected)
        );
        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk, json!({"version":1,"selectedIds":selected}));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn rejected_unknown_duplicate_bambu_and_overflow_never_mutate_file() {
        let path = temp("reject");
        save(&path, &["snapmaker-u1".into()], &none()).unwrap();
        let before = fs::read(&path).unwrap();
        for invalid in [
            vec!["snapmaker-u1".into(), "snapmaker-u1".into()],
            vec!["bambu-lab-a1".into()],
            vec!["unknown-printer".into()],
            vec!["orca-acme-profile-0123456789".into()],
            vec!["snapmaker-u1".into(); 33],
        ] {
            assert!(save(&path, &invalid, &none()).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn corrupt_saved_file_is_not_replaced_or_normalized() {
        let path = temp("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for raw in [
            r#"{"version":2,"selectedIds":[]}"#,
            r#"{"version":1,"selectedIds":null}"#,
            r#"{"version":1,"selectedIds":["qidi-x-plus-5","qidi-x-plus-5"]}"#,
            "{not json",
        ] {
            fs::write(&path, raw).unwrap();
            assert!(read(&path, &none()).is_err());
            assert!(save(&path, &[], &none()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        }
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn absent_dynamic_ids_are_hidden_but_saved_inventory_retained() {
        let path = temp("dynamic");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let dynamic = "orca-flashforge-creator-5-0123456789";
        fs::write(
            &path,
            format!(r#"{{"version":1,"selectedIds":["snapmaker-u1","{dynamic}"]}}"#),
        )
        .unwrap();
        assert_eq!(
            read(&path, &none()).unwrap()["selectedIds"],
            json!(["snapmaker-u1"])
        );
        let saved = save(&path, &["qidi-x-plus-5".into()], &none()).unwrap();
        assert_eq!(saved["selectedIds"], json!(["qidi-x-plus-5"]));
        let disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["selectedIds"], json!(["qidi-x-plus-5", dynamic]));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn resolved_prusa_membership_never_becomes_export_readiness() {
        let path = temp("prusa-membership");
        let id = "prusa-custom-mk4-0123456789".to_string();
        let installed = HashSet::from([id.clone()]);
        assert_eq!(
            save(&path, &[id.clone()], &installed).unwrap()["selectedIds"],
            json!([id])
        );
        // Missing user presets are hidden but never destroyed by a later save.
        assert_eq!(save(&path, &[], &none()).unwrap()["selectedIds"], json!([]));
        let disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["selectedIds"], json!([id]));
        // Once installed again, the membership can be deselected explicitly.
        assert_eq!(
            save(&path, &[], &installed).unwrap()["selectedIds"],
            json!([])
        );
        let disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["selectedIds"], json!([]));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn installed_dynamic_id_is_visible_and_can_be_removed() {
        let path = temp("installed");
        let id = "orca-snapmaker-u1-0123456789".to_string();
        let available = HashSet::from([id.clone()]);
        assert_eq!(
            save(&path, &[id.clone()], &available).unwrap()["selectedIds"],
            json!([id])
        );
        assert_eq!(
            save(&path, &[], &available).unwrap()["selectedIds"],
            json!([])
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
