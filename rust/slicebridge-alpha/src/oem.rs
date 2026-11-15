//! Rust-native, read-only resolution of installed U1 / Creator 5 / CC2 / QIDI OEM presets.
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
    slicer: &'static str,
}
const TARGETS: [Target; 16] = [
    Target {
        id: "c5",
        nozzle: "0.25",
        model: "Flashforge Creator 5",
        name: "Flashforge Creator 5 0.25 nozzle",
        machine: "Flashforge Creator 5 0.25 nozzle",
        process: "0.12mm Standard @FF C5 0.25 nozzle",
        vendor: "Flashforge",
        slicer: "Flash Studio Desktop",
    },
    Target {
        id: "c5",
        nozzle: "0.4",
        model: "Flashforge Creator 5",
        name: "Flashforge Creator 5 0.4 nozzle",
        machine: "Flashforge Creator 5 0.4 nozzle",
        process: "0.20mm Standard @FF C5",
        vendor: "Flashforge",
        slicer: "Flash Studio Desktop",
    },
    Target {
        id: "c5",
        nozzle: "0.6",
        model: "Flashforge Creator 5",
        name: "Flashforge Creator 5 0.6 nozzle",
        machine: "Flashforge Creator 5 0.6 nozzle",
        process: "0.30mm Standard @FF C5 0.6 nozzle",
        vendor: "Flashforge",
        slicer: "Flash Studio Desktop",
    },
    Target {
        id: "c5",
        nozzle: "0.8",
        model: "Flashforge Creator 5",
        name: "Flashforge Creator 5 0.8 nozzle",
        machine: "Flashforge Creator 5 0.8 nozzle",
        process: "0.32mm Standard @FF C5 0.8 nozzle",
        vendor: "Flashforge",
        slicer: "Flash Studio Desktop",
    },
    Target {
        id: "cc2",
        nozzle: "0.2",
        model: "Elegoo Centauri Carbon 2",
        name: "Elegoo Centauri Carbon 2 0.2 nozzle",
        machine: "Elegoo Centauri Carbon 2 0.2 nozzle",
        process: "0.10mm Standard @Elegoo CC2 0.2 nozzle",
        vendor: "Elegoo",
        slicer: "ElegooSlicer",
    },
    Target {
        id: "cc2",
        nozzle: "0.4",
        model: "Elegoo Centauri Carbon 2",
        name: "Elegoo Centauri Carbon 2 0.4 nozzle",
        machine: "Elegoo Centauri Carbon 2 0.4 nozzle",
        process: "0.20mm Standard @Elegoo CC2 0.4 nozzle",
        vendor: "Elegoo",
        slicer: "ElegooSlicer",
    },
    Target {
        id: "cc2",
        nozzle: "0.6",
        model: "Elegoo Centauri Carbon 2",
        name: "Elegoo Centauri Carbon 2 0.6 nozzle",
        machine: "Elegoo Centauri Carbon 2 0.6 nozzle",
        process: "0.30mm Standard @Elegoo CC2 0.6 nozzle",
        vendor: "Elegoo",
        slicer: "ElegooSlicer",
    },
    Target {
        id: "cc2",
        nozzle: "0.8",
        model: "Elegoo Centauri Carbon 2",
        name: "Elegoo Centauri Carbon 2 0.8 nozzle",
        machine: "Elegoo Centauri Carbon 2 0.8 nozzle",
        process: "0.40mm Standard @Elegoo CC2 0.8 nozzle",
        vendor: "Elegoo",
        slicer: "ElegooSlicer",
    },
    Target {
        id: "u1",
        nozzle: "0.2",
        model: "Snapmaker U1",
        name: "Snapmaker U1 (0.2 nozzle)",
        machine: "Snapmaker U1 (0.2 nozzle)",
        process: "0.12mm Standard @Snapmaker U1 (0.2 nozzle)",
        vendor: "Snapmaker",
        slicer: "Snapmaker Orca",
    },
    Target {
        id: "u1",
        nozzle: "0.4",
        model: "Snapmaker U1",
        name: "Snapmaker U1 (0.4 nozzle)",
        machine: "Snapmaker U1 (0.4 nozzle)",
        process: "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
        vendor: "Snapmaker",
        slicer: "Snapmaker Orca",
    },
    Target {
        id: "u1",
        nozzle: "0.6",
        model: "Snapmaker U1",
        name: "Snapmaker U1 (0.6 nozzle)",
        machine: "Snapmaker U1 (0.6 nozzle)",
        process: "0.30mm Standard @Snapmaker U1 (0.6 nozzle)",
        vendor: "Snapmaker",
        slicer: "Snapmaker Orca",
    },
    Target {
        id: "u1",
        nozzle: "0.8",
        model: "Snapmaker U1",
        name: "Snapmaker U1 (0.8 nozzle)",
        machine: "Snapmaker U1 (0.8 nozzle)",
        process: "0.40mm Standard @Snapmaker U1 (0.8 nozzle)",
        vendor: "Snapmaker",
        slicer: "Snapmaker Orca",
    },
    Target {
        id: "qidi",
        nozzle: "0.2",
        model: "X-Plus 5",
        name: "X-Plus 5 0.2 nozzle",
        machine: "X-Plus 5 0.2 nozzle",
        process: "0.10mm Standard @X-Plus 5 0.2 nozzle",
        vendor: "X 5 Series",
        slicer: "QIDIStudio",
    },
    Target {
        id: "qidi",
        nozzle: "0.4",
        model: "X-Plus 5",
        name: "X-Plus 5 0.4 nozzle",
        machine: "X-Plus 5 0.4 nozzle",
        process: "0.20mm Standard @X-Plus 5",
        vendor: "X 5 Series",
        slicer: "QIDIStudio",
    },
    Target {
        id: "qidi",
        nozzle: "0.6",
        model: "X-Plus 5",
        name: "X-Plus 5 0.6 nozzle",
        machine: "X-Plus 5 0.6 nozzle",
        process: "0.30mm Standard @X-Plus 5 0.6 nozzle",
        vendor: "X 5 Series",
        slicer: "QIDIStudio",
    },
    Target {
        id: "qidi",
        nozzle: "0.8",
        model: "X-Plus 5",
        name: "X-Plus 5 0.8 nozzle",
        machine: "X-Plus 5 0.8 nozzle",
        process: "0.40mm Standard @X-Plus 5 0.8 nozzle",
        vendor: "X 5 Series",
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
                "unsupported installed OEM target/nozzle {target}/{nozzle}; supported: U1, Creator5, CC2, QIDI with exact physical variants"
            )
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Location {
    id: &'static str,
    name: &'static str,
    root: PathBuf,
    executable: PathBuf,
}

type AppInstall = (&'static str, &'static str, &'static str, &'static str);

// Read-only, deterministic port of native/presets/options.go::appGroups +
// slicerLocationsForOS. Every vendor app across ALL trusted installation bases
// precedes a generic Orca fallback. Never scan arbitrary disks or Bambu Studio
// for destination profiles.
fn candidate_locations(
    target: &str,
    platform: &str,
    bases: &[PathBuf],
    override_root: Option<&Path>,
) -> Vec<Location> {
    if !["u1", "c5", "cc2", "qidi"].contains(&target) {
        return Vec::new();
    }
    if let Some(root) = override_root {
        return vec![Location {
            id: "system",
            name: "Custom local profile",
            root: root.to_path_buf(),
            executable: PathBuf::new(),
        }];
    }
    let groups: Vec<Vec<AppInstall>> = match (platform, target) {
        ("windows", "u1") => vec![vec![(
            "snapmaker-orca",
            "Snapmaker Orca",
            "Snapmaker_Orca",
            "snapmaker-orca.exe",
        )]],
        ("windows", "c5") => vec![vec![(
            "flash-studio",
            "Flash Studio Desktop",
            "Flashforge/Flash Studio Desktop",
            "flash studio.exe",
        )]],
        ("windows", "cc2") => vec![vec![(
            "elegoo-slicer",
            "ElegooSlicer",
            "ElegooSlicer",
            "elegoo-slicer.exe",
        )]],
        ("windows", "qidi") => vec![
            vec![("qidi-studio", "QIDIStudio", "QIDIStudio", "QIDIStudio.exe")],
            vec![("orca-slicer", "OrcaSlicer", "OrcaSlicer", "orca-slicer.exe")],
        ],
        ("macos", "u1") => vec![vec![(
            "snapmaker-orca",
            "Snapmaker Orca",
            "Snapmaker Orca",
            "",
        )]],
        ("macos", "c5") => vec![vec![
            ("flash-studio", "Flash Studio", "Flash Studio", ""),
            (
                "flash-studio",
                "Flash Studio Desktop",
                "Flash Studio Desktop",
                "",
            ),
        ]],
        ("macos", "cc2") => vec![vec![("elegoo-slicer", "ElegooSlicer", "ElegooSlicer", "")]],
        ("macos", "qidi") => vec![
            vec![("qidi-studio", "QIDIStudio", "QIDIStudio", "")],
            vec![("orca-slicer", "OrcaSlicer", "OrcaSlicer", "")],
        ],
        _ => vec![],
    };
    let mut result = Vec::new();
    for group in groups {
        for base in bases {
            for (id, name, folder, exe) in &group {
                let app = base.join(folder);
                let (root, executable) = if platform == "windows" {
                    (app.join("resources").join("profiles"), app.join(exe))
                } else {
                    let bundle = base.join(format!("{folder}.app"));
                    (
                        bundle.join("Contents").join("Resources").join("profiles"),
                        bundle,
                    )
                };
                result.push(Location {
                    id,
                    name,
                    root,
                    executable,
                });
            }
        }
    }
    result
}

fn locations(item: Target) -> Vec<Location> {
    let (platform, bases) = if cfg!(windows) {
        let mut bases: Vec<PathBuf> = [
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect();
        if bases.is_empty() {
            bases.push(PathBuf::from(r"C:\Program Files"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|x| !x.is_empty()) {
            bases.push(PathBuf::from(local).join("Programs"));
        }
        ("windows", bases)
    } else if cfg!(target_os = "macos") {
        let mut bases = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME").filter(|x| !x.is_empty()) {
            bases.push(PathBuf::from(home).join("Applications"));
        }
        ("macos", bases)
    } else {
        ("unsupported", vec![])
    };
    let override_root = std::env::var_os(format!(
        "BRIDGE_SLICER_PROFILE_{}",
        item.id.to_ascii_uppercase()
    ))
    .filter(|path| !path.is_empty())
    .map(PathBuf::from);
    candidate_locations(item.id, platform, &bases, override_root.as_deref())
}

// The original QIDIStudio Windows install can contain either of these two
// fixed executable names. No arbitrary executable lookup or shell execution.
fn candidate_executable(location: &Location) -> Option<PathBuf> {
    if location.id == "system" {
        return None;
    }
    let trusted = |path: &Path| {
        fs::symlink_metadata(path).is_ok_and(|metadata| {
            !metadata.file_type().is_symlink()
                && ((metadata.is_file() && metadata.len() > 0)
                    || (cfg!(target_os = "macos") && metadata.is_dir()))
        })
    };
    if trusted(&location.executable) {
        return Some(location.executable.clone());
    }
    if cfg!(windows) && location.id == "qidi-studio" {
        let alternate = location.executable.with_file_name("qidi-studio.exe");
        if trusted(&alternate) {
            return Some(alternate);
        }
    }
    None
}

// Go "auto" attempts the next trusted OEM location if an earlier
// installation is incomplete. Presence of an app is not proof of valid
// machine/process/PLA inheritance. Never fall back to generated profiles.
fn resolved_location(
    item: Target,
    slicer_id: &str,
    overrides: &BTreeMap<String, String>,
) -> Result<(Location, Value), String> {
    resolve_from_locations(item, slicer_id, overrides, locations(item))
}

fn resolve_from_locations(
    item: Target,
    slicer_id: &str,
    overrides: &BTreeMap<String, String>,
    locations: Vec<Location>,
) -> Result<(Location, Value), String> {
    // Unlike auto, an explicit saved slicer MUST NOT silently fall back to
    // QIDIStudio, Orca, or another user's application. A custom system root
    // replaces the full location list, exactly as Go does.
    if slicer_id != "auto" && !locations.iter().any(|loc| loc.id == slicer_id) {
        return Err(format!(
            "selected slicer {slicer_id:?} is not a trusted installed profile source for {}",
            item.id
        ));
    }
    let mut failures = Vec::new();
    for location in locations
        .into_iter()
        .filter(|loc| slicer_id == "auto" || loc.id == slicer_id)
    {
        if !location.root.is_absolute() {
            failures.push(format!("invalid relative OEM root for {}", location.name));
            continue;
        }
        if location.id != "system" && candidate_executable(&location).is_none() {
            continue;
        }
        if !location.root.join(item.vendor).is_dir() {
            continue;
        }
        match load_index(&location.root, item.vendor)
            .and_then(|index| resolve_with_index(item, &index, &location.root, overrides))
        {
            Ok(mut bundle) => {
                // The actual vendor-fork identity belongs in the receipt, not
                // the target's default slicer label.
                bundle["slicer"] = json!(location.name);
                bundle["slicer_id"] = json!(location.id);
                return Ok((location, bundle));
            }
            Err(error) => failures.push(format!("{}: {error}", location.name)),
        }
    }
    if failures.is_empty() {
        Err(format!(
            "installed {} source profile {slicer_id:?} not found",
            item.slicer
        ))
    } else {
        Err(format!(
            "no complete installed {} source profile: {}",
            item.slicer,
            failures.join("; ")
        ))
    }
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
        let vendor_dir = root.join(vendor).join(kind);
        if !vendor_dir.is_dir() {
            return Err(format!("installed OEM {kind} directory absent"));
        }
        // Go presets.readCatalog includes Flashforge's installed
        // OrcaFilamentLibrary, which owns inherited Generic @System leaves.
        // No generated or cross-vendor profile is ever fabricated.
        let mut directories = vec![vendor_dir];
        if vendor == "Flashforge" {
            let library_dir = root.join("OrcaFilamentLibrary").join(kind);
            if library_dir.is_dir() {
                directories.push(library_dir);
            }
        }
        let mut count = 0;
        for dir in directories {
            let mut pending = vec![dir];
            while let Some(folder) = pending.pop() {
                for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let ftype = entry.file_type().map_err(|e| e.to_string())?;
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
    let marker = match target.id {
        "u1" => "@U1",
        "c5" => "@FF C5",
        "cc2" => "@ECC2",
        "qidi" => "@Qidi X-Plus 5",
        _ => return false,
    };
    let qualified_leaf = target.id == "u1"
        && target.nozzle == "0.4"
        && (name == "Generic PLA High Speed" || name == "Generic PETG");
    if (!name.contains(marker) && !qualified_leaf)
        || (target.id == "c5" && name.contains("@FF C5P"))
    {
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
        ("u1", "PLA") if target.nozzle == "0.4" => Some("Generic PLA High Speed".into()),
        ("u1", "PETG") if target.nozzle == "0.4" => Some("Generic PETG".into()),
        ("u1", type_name) => Some(format!("Generic {type_name} @U1 {} nozzle", target.nozzle)),
        ("c5", kind @ ("PLA" | "PETG" | "ABS" | "ASA")) => {
            let suffix = if target.nozzle == "0.25" {
                " 0.25 nozzle"
            } else {
                ""
            };
            Some(format!("Flashforge {kind} Basic @FF C5{suffix}"))
        }
        ("cc2", "TPU") => Some("Elegoo TPU 95A @ECC2".into()),
        ("cc2", kind) => Some(format!("Elegoo {kind} @ECC2")),
        ("qidi", type_name) => Some(format!(
            "Generic {type_name} @Qidi X-Plus 5 {} nozzle",
            target.nozzle
        )),
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
    resolve_configured(target, nozzle, "auto", overrides)
}

pub fn resolve_configured(
    target: &str,
    nozzle: &str,
    slicer_id: &str,
    overrides: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let item = spec(target, nozzle)?;
    let (_, bundle) = resolved_location(item, slicer_id, overrides)?;
    Ok(bundle)
}

pub fn installed_executable(target: &str, nozzle: &str) -> Result<PathBuf, String> {
    installed_executable_configured(target, nozzle, "auto")
}

pub fn installed_executable_configured(
    target: &str,
    nozzle: &str,
    slicer_id: &str,
) -> Result<PathBuf, String> {
    let item = spec(target, nozzle)?;
    let (location, _) = resolved_location(item, slicer_id, &BTreeMap::new())?;
    // An owner-supplied profile-root override is NOT an executable.
    candidate_executable(&location)
        .ok_or_else(|| "selected system profile has no trusted slicer executable".into())
}

pub fn choices(target: &str, nozzle: &str, kind: &str) -> Result<Vec<String>, String> {
    choices_configured(target, nozzle, "auto", kind)
}

pub fn choices_configured(
    target: &str,
    nozzle: &str,
    slicer_id: &str,
    kind: &str,
) -> Result<Vec<String>, String> {
    let item = spec(target, nozzle)?;
    if !["PLA", "PETG", "ABS", "ASA", "TPU"].contains(&kind) {
        return Err("material type unsupported".into());
    }
    let (location, _) = resolved_location(item, slicer_id, &BTreeMap::new())?;
    let index = load_index(&location.root, item.vendor)?;
    Ok(material_choices(&index, item, kind))
}
// Expose the exact trusted slicer IDs and their installed readiness to GPUI.
// With BRIDGE_SLICER_PROFILE_* set, 'system' is the ONLY valid installed source
// (there is deliberately no executable path in that override).
pub fn slicer_options(target: &str, nozzle: &str) -> Result<Vec<Value>, String> {
    let item = spec(target, nozzle)?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for loc in locations(item) {
        if seen.insert(loc.id) {
            let installed = resolve_configured(target, nozzle, loc.id, &BTreeMap::new()).is_ok();
            rows.push(json!({
                "id": loc.id,
                "name": loc.name,
                "installed": installed
            }));
        }
    }
    Ok(rows)
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
    json!({"profiles":rows,"source":"read-only installed OEM resources","scope":"u1/c5/cc2/qidi exact physical nozzle variants"})
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
    fn qidi_installed_slicer_candidate_order_matches_go_across_bases() {
        let windows = [
            PathBuf::from("C:/Program Files"),
            PathBuf::from("C:/Program Files (x86)"),
            PathBuf::from("C:/Users/Tester/AppData/Local/Programs"),
        ];
        let candidates = candidate_locations("qidi", "windows", &windows, None);
        let ids = candidates.iter().map(|loc| loc.id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "qidi-studio",
                "qidi-studio",
                "qidi-studio",
                "orca-slicer",
                "orca-slicer",
                "orca-slicer",
            ]
        );
        assert_eq!(
            candidates[0].root,
            windows[0].join("QIDIStudio/resources/profiles")
        );
        assert_eq!(
            candidates[3].root,
            windows[0].join("OrcaSlicer/resources/profiles")
        );
        let bases = [
            PathBuf::from("/Applications"),
            PathBuf::from("/Users/tester/Applications"),
        ];
        let mac = candidate_locations("qidi", "macos", &bases, None);
        assert_eq!(
            mac.iter().map(|loc| loc.id).collect::<Vec<_>>(),
            vec!["qidi-studio", "qidi-studio", "orca-slicer", "orca-slicer"]
        );
        assert_eq!(
            mac[1].executable,
            PathBuf::from("/Users/tester/Applications/QIDIStudio.app")
        );
        let flash = candidate_locations("c5", "macos", &bases, None);
        assert_eq!(flash.len(), 4);
        assert_eq!(
            flash[1].executable,
            PathBuf::from("/Applications/Flash Studio Desktop.app")
        );
        let root = Path::new("/read-only/explicit-profiles");
        let custom = candidate_locations("qidi", "windows", &windows, Some(root));
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].id, "system");
        assert_eq!(custom[0].root, root);
        assert!(custom[0].executable.as_os_str().is_empty());
        assert!(candidate_locations("../qidi", "windows", &windows, None).is_empty());
        assert!(candidate_locations("qidi", "linux", &windows, None).is_empty());
    }

    #[test]
    fn explicit_installed_slicer_never_falls_back_to_other_app() {
        // Both .exe and vendor profiles here are MOCK files under a
        // disposable test directory. No program or slicer is ever launched.
        let root = temp_dir();
        let spec = spec("qidi", "0.4").unwrap();
        let orca_root = root.join("orca-profiles");
        make_profile(
            &orca_root,
            spec.vendor,
            "machine",
            &format!("{}.json", spec.machine),
            json!({
                "name":spec.machine, "printer_model":spec.model,
                "nozzle_diameter":["0.4"]
            }),
        );
        make_profile(
            &orca_root,
            spec.vendor,
            "process",
            &format!("{}.json", spec.process),
            json!({"name":spec.process, "layer_height":"0.2"}),
        );
        let pla = preferred(spec, "PLA").unwrap();
        make_profile(
            &orca_root,
            spec.vendor,
            "filament",
            &format!("{pla}.json"),
            json!({
                "name":pla,"filament_type":["PLA"],
                "compatible_printers":[spec.machine]
            }),
        );
        let installed_orca = root.join("orca-slicer.exe");
        fs::write(&installed_orca, b"QA fake executable -- never run").unwrap();
        let installed_vendor = root.join("qidi-studio.exe");
        fs::write(&installed_vendor, b"QA fake executable -- never run").unwrap();
        let vendor = Location {
            id: "qidi-studio",
            name: "QIDIStudio",
            root: root.join("missing-qidi-profiles"),
            executable: installed_vendor,
        };
        let orca = Location {
            id: "orca-slicer",
            name: "OrcaSlicer",
            root: orca_root,
            executable: installed_orca,
        };
        let installed = vec![vendor.clone(), orca.clone()];
        let (_, auto) =
            resolve_from_locations(spec, "auto", &BTreeMap::new(), installed.clone()).unwrap();
        assert_eq!(auto["slicer_id"], "orca-slicer");
        assert_eq!(auto["slicer"], "OrcaSlicer");
        assert_eq!(auto["target"], "qidi");
        let (_, explicit_orca) =
            resolve_from_locations(spec, "orca-slicer", &BTreeMap::new(), installed.clone())
                .unwrap();
        assert_eq!(explicit_orca["slicer_id"], "orca-slicer");
        assert!(
            resolve_from_locations(spec, "qidi-studio", &BTreeMap::new(), installed.clone())
                .is_err()
        );
        assert!(resolve_from_locations(spec, "system", &BTreeMap::new(), installed).is_err());
        assert!(resolve_from_locations(
            spec,
            "auto",
            &BTreeMap::new(),
            vec![Location {
                id: "system",
                name: "Custom local profile",
                root: root.join("missing-custom"),
                executable: PathBuf::new()
            }]
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn all_sixteen_nozzle_presets_have_exact_vendor_processes() {
        assert_eq!(TARGETS.len(), 16);
        for (id, nozzles) in [
            ("u1", ["0.2", "0.4", "0.6", "0.8"]),
            ("c5", ["0.25", "0.4", "0.6", "0.8"]),
            ("cc2", ["0.2", "0.4", "0.6", "0.8"]),
            ("qidi", ["0.2", "0.4", "0.6", "0.8"]),
        ] {
            for nozzle in nozzles {
                let target = spec(id, nozzle).unwrap();
                assert_eq!(target.nozzle, nozzle);
                assert!(target.machine.contains(nozzle));
                assert!(
                    target.process.contains(nozzle)
                        || ((id == "qidi" || id == "c5") && nozzle == "0.4")
                );
                assert!(preferred(target, "PLA").is_some());
                assert!(preferred(target, "PETG").is_some());
            }
        }
        assert_eq!(
            spec("c5", "0.25").unwrap().machine,
            "Flashforge Creator 5 0.25 nozzle"
        );
        assert_eq!(
            spec("c5", "0.8").unwrap().process,
            "0.32mm Standard @FF C5 0.8 nozzle"
        );
        assert_eq!(
            spec("cc2", "0.4").unwrap().model,
            "Elegoo Centauri Carbon 2"
        );
        assert_eq!(
            preferred(spec("cc2", "0.4").unwrap(), "TPU").unwrap(),
            "Elegoo TPU 95A @ECC2"
        );
        assert!(preferred(spec("c5", "0.4").unwrap(), "TPU").is_none());
        assert!(spec("u1", "0.25").is_err());
        assert!(spec("c5", "0.2").is_err());
        assert!(spec("other", "0.4").is_err());
    }
    // Only synthetic OEM JSON lives in the temporary test directory.
    // The real Flash Studio and ElegooSlicer profiles are read-only.
    fn make_profile(root: &Path, vendor: &str, kind: &str, file: &str, value: Value) {
        let dir = root.join(vendor).join(kind);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn creator5_resolves_exact_installed_machine_and_inherited_filament_library() {
        let root = temp_dir();
        let s = spec("c5", "0.4").unwrap();
        make_profile(
            &root,
            "Flashforge",
            "machine",
            &format!("{}.json", s.machine),
            json!({"name":s.machine, "printer_model":s.model,
                "nozzle_diameter":["0.4","0.4","0.4","0.4"]}),
        );
        make_profile(
            &root,
            "Flashforge",
            "process",
            &format!("{}.json", s.process),
            json!({"name":s.process,"layer_height":"0.2",
                "compatible_printers":[s.machine]}),
        );
        make_profile(
            &root,
            "OrcaFilamentLibrary",
            "filament",
            "Generic PLA @System.json",
            json!({"name":"Generic PLA @System","filament_type":["PLA"]}),
        );
        make_profile(
            &root,
            "Flashforge",
            "filament",
            "Flashforge PLA Basic @FF C5.json",
            json!({"name":"Flashforge PLA Basic @FF C5",
                "inherits":"Generic PLA @System",
                "compatible_printers":[s.machine]}),
        );
        let index = load_index(&root, "Flashforge").unwrap();
        let bundle = resolve_with_index(s, &index, &root, &BTreeMap::new()).unwrap();
        assert_eq!(bundle["target"], "c5");
        assert_eq!(
            bundle["profile"]["project_settings"]["printer_model"],
            "Flashforge Creator 5"
        );
        assert_eq!(
            bundle["profile"]["filament_profiles"]["PLA"],
            "Flashforge PLA Basic @FF C5"
        );
        assert_eq!(
            bundle["material_settings"]["PLA"]["filament_type"],
            json!(["PLA"])
        );
        let spoofed_pro = Preset {
            file: "untrusted-pro.json".into(),
            stem: "Flashforge PLA Basic @FF C5P".into(),
            settings: serde_json::from_value(json!({
                "compatible_printers":[s.machine],"filament_type":["PLA"]
            }))
            .unwrap(),
        };
        assert!(!material_allowed(
            s,
            s.machine,
            &spoofed_pro.stem,
            &spoofed_pro
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cc2_resolves_exact_vendor_filament_and_refuses_wrong_identity() {
        let root = temp_dir();
        let s = spec("cc2", "0.4").unwrap();
        make_profile(
            &root,
            "Elegoo",
            "machine",
            &format!("{}.json", s.machine),
            json!({"name":s.machine,"printer_model":s.model,"nozzle_diameter":["0.4"]}),
        );
        make_profile(
            &root,
            "Elegoo",
            "process",
            &format!("{}.json", s.process),
            json!({"name":s.process,"compatible_printers":[s.machine]}),
        );
        make_profile(
            &root,
            "Elegoo",
            "filament",
            "Elegoo PLA @base.json",
            json!({"name":"Elegoo PLA @base","filament_type":["PLA"]}),
        );
        make_profile(
            &root,
            "Elegoo",
            "filament",
            "Elegoo PLA @ECC2.json",
            json!({"name":"Elegoo PLA @ECC2","inherits":"Elegoo PLA @base",
                "compatible_printers":[s.machine]}),
        );
        let index = load_index(&root, "Elegoo").unwrap();
        let bundle = resolve_with_index(s, &index, &root, &BTreeMap::new()).unwrap();
        assert_eq!(bundle["target"], "cc2");
        assert_eq!(
            bundle["profile"]["filament_profiles"]["PLA"],
            "Elegoo PLA @ECC2"
        );
        assert_eq!(
            bundle["profile"]["project_settings"]["printer_settings_id"],
            s.machine
        );
        make_profile(
            &root,
            "Elegoo",
            "machine",
            &format!("{}.json", s.machine),
            json!({"name":s.machine,"printer_model":"DIFFERENT MODEL",
                "nozzle_diameter":["0.4"]}),
        );
        let broken_index = load_index(&root, "Elegoo").unwrap();
        assert!(
            resolve_with_index(s, &broken_index, &root, &BTreeMap::new())
                .unwrap_err()
                .contains("identity")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vendor_04_petg_leaf_is_not_reused_for_other_nozzles() {
        let settings: Map<String, Value> = serde_json::from_value(json!({
            "compatible_printers":["Snapmaker U1 (0.4 nozzle)"],
            "filament_type":["PETG"]
        }))
        .unwrap();
        let leaf = Preset {
            file: "leaf.json".into(),
            stem: "Generic PETG".into(),
            settings,
        };
        assert!(material_allowed(
            spec("u1", "0.4").unwrap(),
            "Snapmaker U1 (0.4 nozzle)",
            "Generic PETG",
            &leaf
        ));
        assert!(!material_allowed(
            spec("u1", "0.6").unwrap(),
            "Snapmaker U1 (0.6 nozzle)",
            "Generic PETG",
            &leaf
        ));
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
        let choices = material_choices(&ix, spec("qidi", "0.4").unwrap(), "PLA");
        assert_eq!(choices, vec!["Generic PLA @Qidi X-Plus 5 0.4 nozzle"]);
        assert!(material_choices(&ix, spec("qidi", "0.4").unwrap(), "PETG").is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
