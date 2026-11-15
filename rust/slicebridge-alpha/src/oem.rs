//! Rust-native, read-only resolution of installed Snapmaker/QIDI OEM presets.
//! Exact vendor leafs and their inheritance supply the conversion engine;
//! no profile payload is synthesized or written back to slicer installations.
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct Target {
    id: &'static str,
    model: &'static str,
    name: &'static str,
    nozzle: &'static str,
    machine: &'static str,
    process: &'static str,
    vendor: &'static str,
    app: &'static str,
    exe: &'static str,
    slicer: &'static str,
}
const TARGETS: [Target; 2] = [
    Target {
        id: "u1",
        model: "Snapmaker U1",
        name: "Snapmaker U1 (0.4 nozzle)",
        nozzle: "0.4",
        machine: "Snapmaker U1 (0.4 nozzle)",
        process: "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
        vendor: "Snapmaker",
        app: "Snapmaker_Orca",
        exe: "snapmaker-orca.exe",
        slicer: "Snapmaker Orca",
    },
    Target {
        id: "qidi",
        model: "X-Plus 5",
        name: "X-Plus 5 0.4 nozzle",
        nozzle: "0.4",
        machine: "X-Plus 5 0.4 nozzle",
        process: "0.20mm Standard @X-Plus 5",
        vendor: "X 5 Series",
        app: "QIDIStudio",
        exe: "qidi-studio.exe",
        slicer: "QIDIStudio",
    },
];

fn spec(target: &str, nozzle: &str) -> Result<Target, String> {
    TARGETS
        .iter()
        .copied()
        .find(|item| item.id == target && item.nozzle == nozzle)
        .ok_or_else(|| {
            format!(
                "only installed u1/qidi 0.4mm OEM targets are implemented (got {target}/{nozzle})"
            )
        })
}

#[derive(Clone)]
struct Location {
    root: PathBuf,
    executable: PathBuf,
}

fn locations(item: Target) -> Vec<Location> {
    let mut result = Vec::new();
    if cfg!(windows) {
        let mut bases: Vec<PathBuf> = [
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect();
        if bases.is_empty() {
            bases.push(PathBuf::from(r"C:\Program Files"));
        }
        for base in bases {
            let app = base.join(item.app);
            result.push(Location {
                executable: app.join(item.exe),
                root: app.join("resources").join("profiles"),
            });
        }
    } else if cfg!(target_os = "macos") {
        let applications = [PathBuf::from("/Applications"), dirs_user_applications()];
        let app = if item.id == "u1" {
            "Snapmaker Orca.app"
        } else {
            "QIDIStudio.app"
        };
        for base in applications {
            let bundle = base.join(app);
            result.push(Location {
                executable: bundle.clone(),
                root: bundle.join("Contents").join("Resources").join("profiles"),
            });
        }
    }
    result
}
#[cfg(target_os = "macos")]
fn dirs_user_applications() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Applications")
}
#[cfg(not(target_os = "macos"))]
fn dirs_user_applications() -> PathBuf {
    PathBuf::new()
}

fn installed(item: Target) -> Result<Location, String> {
    locations(item)
        .into_iter()
        .find(|loc| loc.executable.exists() && loc.root.join(item.vendor).is_dir())
        .ok_or_else(|| {
            format!(
                "installed {} executable and OEM profiles not found",
                item.slicer
            )
        })
}

#[derive(Clone)]
struct Preset {
    file: PathBuf,
    stem: String,
    settings: Map<String, Value>,
}
type Index = BTreeMap<String, Vec<Preset>>;
const OMIT: [&str; 12] = [
    "type",
    "name",
    "from",
    "inherits",
    "setting_id",
    "instantiation",
    "renamed_from",
    "compatible_printers",
    "compatible_printers_condition",
    "compatible_prints",
    "compatible_prints_condition",
    "upward_compatible_machine",
];

fn parse_vendor_json(path: &Path) -> Result<Map<String, Value>, String> {
    let bytes = fs::read(path).map_err(|e| format!("OEM read {}: {e}", path.display()))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("OEM preset JSON exceeds 2 MiB".into());
    }
    let content = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let value: Value =
        serde_json::from_slice(content).map_err(|e| format!("OEM JSON {}: {e}", path.display()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("OEM profile is not an object: {}", path.display()))
}
fn insert_index(index: &mut Index, kind: &str, file: PathBuf) -> Result<(), String> {
    if !file
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return Ok(());
    }
    let settings = parse_vendor_json(&file)?;
    if settings.is_empty() {
        return Ok(());
    }
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "OEM preset filename is not UTF-8".to_string())?
        .to_string();
    let mut names = vec![stem.clone()];
    if let Some(name) = settings.get("name").and_then(Value::as_str) {
        if !name.is_empty() && name != stem {
            names.push(name.to_string());
        }
    }
    for name in names {
        index
            .entry(format!("{kind}:{name}"))
            .or_default()
            .push(Preset {
                file: file.clone(),
                stem: stem.clone(),
                settings: settings.clone(),
            });
    }
    Ok(())
}
fn load_index(root: &Path, vendor: &str) -> Result<Index, String> {
    let mut index = Index::new();
    for kind in ["machine", "process", "filament"] {
        let dir = root.join(vendor).join(kind);
        if !dir.is_dir() {
            return Err(format!("installed OEM {kind} directory absent"));
        }
        let mut pending = vec![dir];
        let mut count = 0;
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let ftype = entry.file_type().map_err(|e| e.to_string())?;
                // No need to follow symlinked installation children.
                if ftype.is_symlink() {
                    continue;
                }
                if ftype.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if ftype.is_file() {
                    count += 1;
                    if count > 2000 {
                        return Err("too many installed OEM files".into());
                    }
                    insert_index(&mut index, kind, entry.path())?;
                }
            }
        }
    }
    for entries in index.values_mut() {
        entries.sort_by(|a, b| a.file.cmp(&b.file));
    }
    Ok(index)
}
fn raw_preset<'a>(index: &'a Index, kind: &str, name: &str) -> Result<&'a Preset, String> {
    let list = index
        .get(&format!("{kind}:{name}"))
        .ok_or_else(|| format!("installed {kind} preset {name:?} missing"))?;
    Ok(list
        .iter()
        .find(|entry| entry.stem == name)
        .unwrap_or(&list[0]))
}
fn flatten(index: &Index, kind: &str, name: &str) -> Result<Map<String, Value>, String> {
    flatten_step(index, kind, name, &mut BTreeSet::new())
}
fn flatten_step(
    index: &Index,
    kind: &str,
    name: &str,
    chain: &mut BTreeSet<PathBuf>,
) -> Result<Map<String, Value>, String> {
    if chain.len() > 32 {
        return Err("OEM inheritance depth exceeded".into());
    }
    let current = raw_preset(index, kind, name)?;
    if !chain.insert(current.file.clone()) {
        return Err("OEM inheritance cycle".into());
    }
    let mut merged = if let Some(parent) = current.settings.get("inherits").and_then(Value::as_str)
    {
        if parent.is_empty() {
            Map::new()
        } else {
            flatten_step(index, kind, parent, chain)?
        }
    } else {
        Map::new()
    };
    for (key, value) in &current.settings {
        if !OMIT.contains(&key.as_str()) {
            merged.insert(key.clone(), value.clone());
        }
    }
    chain.remove(&current.file);
    Ok(merged)
}
fn nozzle_matches(settings: &Map<String, Value>, expected: &str) -> bool {
    let Some(arr) = settings.get("nozzle_diameter").and_then(Value::as_array) else {
        return false;
    };
    !arr.is_empty()
        && arr.len() <= 16
        && arr.iter().all(|v| {
            v.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v.as_f64())
                .is_some_and(|n| (n - expected.parse::<f64>().unwrap_or(0.0)).abs() < 1e-6)
        })
}
fn material_type(settings: &Map<String, Value>) -> Option<&str> {
    settings.get("filament_type")?.as_array()?.first()?.as_str()
}
fn material_allowed(target: Target, machine: &str, name: &str, raw: &Preset) -> bool {
    let marker = if target.id == "u1" {
        "@U1"
    } else {
        "@Qidi X-Plus 5"
    };
    let qualified_leaf =
        target.id == "u1" && (name == "Generic PLA High Speed" || name == "Generic PETG");
    if !name.contains(marker) && !qualified_leaf {
        return false;
    }
    for n in ["0.2", "0.25", "0.4", "0.6", "0.8"] {
        if name.contains(&format!("{n} nozzle")) && n != target.nozzle {
            return false;
        }
    }
    // A canonical manufacturer leaf must not claim an incompatible printer.
    match raw
        .settings
        .get("compatible_printers")
        .and_then(Value::as_array)
    {
        Some(list) => list.iter().any(|s| s.as_str() == Some(machine)),
        None => !qualified_leaf, // leaf without explicit machine compatibility is never upgraded
    }
}
fn material_choices(index: &Index, target: Target, kind: &str) -> Vec<String> {
    let mut names = Vec::new();
    for key in index.keys().filter(|k| k.starts_with("filament:")) {
        let name = &key["filament:".len()..];
        let Ok(raw) = raw_preset(index, "filament", name) else {
            continue;
        };
        if !material_allowed(target, target.machine, name, raw) {
            continue;
        }
        let Ok(flattened) = flatten(index, "filament", name) else {
            continue;
        };
        if material_type(&flattened) == Some(kind) {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}
fn preferred(target: Target, kind: &str) -> Option<String> {
    match (target.id, kind) {
        ("u1", "PLA") => Some("Generic PLA High Speed".into()),
        ("u1", "PETG") => Some("Generic PETG".into()),
        ("qidi", type_name) => Some(format!("Generic {type_name} @Qidi X-Plus 5 0.4 nozzle")),
        _ => None,
    }
}
fn resolve_with_index(
    target: Target,
    index: &Index,
    root: &Path,
    overrides: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let machine = flatten(index, "machine", target.machine)?;
    if machine.get("printer_model").and_then(Value::as_str) != Some(target.model) {
        return Err("installed machine identity differs from expected OEM model".into());
    }
    if !nozzle_matches(&machine, target.nozzle) {
        return Err("installed machine nozzle diameter differs from selected nozzle".into());
    }
    let process = flatten(index, "process", target.process)?;
    let mut material_settings = Map::new();
    let mut material_profiles = Map::new();
    for kind in ["PLA", "PETG", "ABS", "ASA", "TPU"] {
        let choices = material_choices(index, target, kind);
        let selected = if let Some(selected) = overrides.get(kind) {
            if !choices.contains(selected) {
                return Err(format!(
                    "selected {kind} preset is not installed/compatible: {selected}"
                ));
            }
            Some(selected.clone())
        } else {
            preferred(target, kind)
                .filter(|p| choices.contains(p))
                .or_else(|| choices.first().cloned())
        };
        if let Some(name) = selected {
            let flattened = flatten(index, "filament", &name)?;
            if material_type(&flattened) != Some(kind) {
                return Err("OEM material mismatch".into());
            }
            material_profiles.insert(kind.into(), Value::String(name));
            material_settings.insert(kind.into(), Value::Object(flattened));
        }
    }
    let pla_name = material_profiles
        .get("PLA")
        .and_then(Value::as_str)
        .ok_or("installed OEM has no validated PLA profile")?
        .to_string();
    let pla = material_settings["PLA"]
        .as_object()
        .ok_or("PLA preset invalid")?;
    let mut project = Map::new();
    for layer in [&machine, &process, pla] {
        for (k, v) in layer {
            project.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in &machine {
        if k.starts_with("machine_")
            || [
                "nozzle_diameter",
                "change_filament_gcode",
                "layer_change_gcode",
                "before_layer_change_gcode",
            ]
            .contains(&k.as_str())
        {
            project.insert(k.clone(), v.clone());
        }
    }
    project.insert("printer_model".into(), Value::String(target.model.into()));
    project.insert(
        "printer_settings_id".into(),
        Value::String(target.machine.into()),
    );
    project.insert(
        "print_settings_id".into(),
        Value::String(target.process.into()),
    );
    project.insert("filament_settings_id".into(), json!([pla_name.clone()]));
    project.insert("filament_type".into(), json!(["PLA"]));
    project.insert("filament_colour".into(), json!(["#FFFFFF"]));
    let root_display = root.to_string_lossy().to_string();
    Ok(json!({
        "target": target.id,
        "slicer": target.slicer,
        "root": root_display,
        "nozzle": target.nozzle,
        "profile": {
            "name": target.id,
            "display_name": target.model,
            "printer_model_id": target.model,
            "filament_slots": 4,
            "filament_profiles": material_profiles,
            "default_filament_profile": pla_name,
            "project_settings": project
        },
        "material_settings": material_settings
    }))
}
pub fn resolve(
    target: &str,
    nozzle: &str,
    overrides: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let s = spec(target, nozzle)?;
    let loc = installed(s)?;
    let index = load_index(&loc.root, s.vendor)?;
    resolve_with_index(s, &index, &loc.root, overrides)
}
pub fn installed_executable(target: &str, nozzle: &str) -> Result<PathBuf, String> {
    let target = spec(target, nozzle)?;
    let location = installed(target)?;
    Ok(location.executable)
}

pub fn choices(target: &str, nozzle: &str, kind: &str) -> Result<Vec<String>, String> {
    let spec = spec(target, nozzle)?;
    if !["PLA", "PETG", "ABS", "ASA", "TPU"].contains(&kind) {
        return Err("material type unsupported".into());
    }
    let loc = installed(spec)?;
    let index = load_index(&loc.root, spec.vendor)?;
    Ok(material_choices(&index, spec, kind))
}
pub fn discover() -> Value {
    let mut rows = Vec::new();
    for t in TARGETS {
        let result = resolve(t.id, t.nozzle, &BTreeMap::new());
        let record = match result {
            Ok(ref bundle) => json!({
                "target":t.id,"nozzle":t.nozzle,"model":t.model,"name":t.name,"slicer":t.slicer,
                "ready":true,
                "materials":bundle["profile"]["filament_profiles"],
            }),
            Err(e) => json!({
                "target":t.id,"nozzle":t.nozzle,"model":t.model,"name":t.name,"slicer":t.slicer,
                "ready":false,"error":e
            }),
        };
        rows.push(record);
    }
    json!({"profiles":rows,"source":"read-only installed OEM resources","scope":"u1/qidi 0.4mm"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "slicebridge-oem-test-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn vendor_bom_and_inheritance_merge_without_metadata() {
        let root = temp_dir();
        let dir = root.join("Snapmaker").join("machine");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("base.json"), br#"{"printer_model":"Snapmaker U1","nozzle_diameter":["0.4"],"machine_start_gcode":"gcode"}"#).unwrap();
        let mut raw = vec![0xef, 0xbb, 0xbf];
        raw.extend_from_slice(br#"{"name":"test leaf","inherits":"base","printer_model":"Snapmaker U1","nozzle_diameter":["0.4"],"machine_end_gcode":"end"}"#);
        fs::write(dir.join("child.json"), raw).unwrap();
        let mut index = Index::new();
        insert_index(&mut index, "machine", dir.join("base.json")).unwrap();
        insert_index(&mut index, "machine", dir.join("child.json")).unwrap();
        let merged = flatten(&index, "machine", "test leaf").unwrap();
        assert_eq!(merged["machine_start_gcode"], "gcode");
        assert_eq!(merged["machine_end_gcode"], "end");
        assert!(!merged.contains_key("inherits"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn inheritance_cycle_is_rejected() {
        let root = temp_dir();
        fs::write(root.join("a.json"), r#"{"inherits":"b"}"#).unwrap();
        fs::write(root.join("b.json"), r#"{"inherits":"a"}"#).unwrap();
        let mut index = Index::new();
        insert_index(&mut index, "filament", root.join("a.json")).unwrap();
        insert_index(&mut index, "filament", root.join("b.json")).unwrap();
        assert!(flatten(&index, "filament", "a")
            .unwrap_err()
            .contains("cycle"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn material_picker_refuses_wrong_machine_and_type() {
        let root = temp_dir();
        let paths = [
            (
                "a.json",
                r#"{"name":"Generic PLA @Qidi X-Plus 5 0.4 nozzle","filament_type":["PLA"],"compatible_printers":["X-Plus 5 0.4 nozzle"]}"#,
            ),
            (
                "b.json",
                r#"{"name":"Generic PETG @Qidi X-Plus 5 0.4 nozzle","filament_type":["PETG"],"compatible_printers":["X-Plus 5 0.6 nozzle"]}"#,
            ),
            (
                "c.json",
                r#"{"name":"Generic PLA @Qidi X-Plus 5 0.6 nozzle","filament_type":["PLA"],"compatible_printers":["X-Plus 5 0.4 nozzle"]}"#,
            ),
        ];
        let mut ix = Index::new();
        for (name, data) in paths {
            let path = root.join(name);
            fs::write(&path, data).unwrap();
            insert_index(&mut ix, "filament", path).unwrap();
        }
        let choices = material_choices(&ix, TARGETS[1], "PLA");
        assert_eq!(choices, vec!["Generic PLA @Qidi X-Plus 5 0.4 nozzle"]);
        assert!(material_choices(&ix, TARGETS[1], "PETG").is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
