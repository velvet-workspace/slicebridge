//! Rust port of native/presets/orca.go: bounded, read-only discovery of installed
//! Orca-family machine presets (Snapmaker Orca, Flash Studio, ElegooSlicer,
//! QIDIStudio, Creality Print, Anycubic Slicer Next, OrcaSlicer).
//!
//! A machine is offered only when its machine, a compatible installed process and
//! a PLA filament all validate and explicitly list that machine. An app folder
//! alone is never ready. Nothing is written, converted, launched, or read from
//! outside the trusted install roots. Symlinks are never followed.
//!
//! Not yet registered: lib.rs must declare this module (`pub mod orca_profiles;`).
use crate::oem::{self, Index, Location, Preset};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

const MAX_REJECTIONS: usize = 50;
const MAX_TOOLS: usize = 8;
const MAX_VENDORS: usize = 64;
const MAX_MACHINE_FILES: usize = 512;
const MAX_CATALOG_FILES: usize = 20_000;
const MAX_PROFILE_BYTES: u64 = 4 << 20;
const MAX_DEPTH: usize = 4;
const MAX_COMPATIBILITY_HOPS: usize = 8;
const MATERIALS: [&str; 5] = ["PLA", "PETG", "ABS", "ASA", "TPU"];
const NOZZLES: [&str; 5] = ["0.2", "0.25", "0.4", "0.6", "0.8"];
const IO_ERROR: &str = "I/O error";

/// One installed machine that resolved completely. Its ID is derived from the
/// machine file location and must be re-resolved before every use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrcaTarget {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) slicer: String,
    pub(crate) vendor: String,
    pub(crate) machine: String,
    pub(crate) model: String,
    pub(crate) nozzle: String,
    pub(crate) physical_tools: usize,
    pub(crate) materials: Vec<String>,
}

/// Explains why an installed machine preset is not offered. Carries no path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrcaRejection {
    pub(crate) vendor: String,
    pub(crate) machine: String,
    pub(crate) slicer: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrcaDiscovery {
    pub(crate) targets: Vec<OrcaTarget>,
    pub(crate) rejected: Vec<OrcaRejection>,
}

/// Trusted application that owns an installed profile root. Only the fixed
/// executable names in the app table are ever returned; nothing is launched here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrcaSlicerApp {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) executable: PathBuf,
}

struct Candidate {
    target: OrcaTarget,
    bundle: Value,
    root: PathBuf,
}

enum MachineError {
    Abstract,
    Rejected(String),
}

fn rejected(reason: impl Into<String>) -> MachineError {
    MachineError::Rejected(reason.into())
}

type ByMachine = BTreeMap<String, BTreeMap<String, Vec<String>>>;

struct AppInstall {
    id: &'static str,
    name: &'static str,
    folder: &'static str,
    exe: &'static str,
}

// Priority order from native/presets/options.go::orcaFamilyApps. Vendor forks
// come first and generic OrcaSlicer last. Windows folders use '/' separators.
static WINDOWS_APPS: [AppInstall; 9] = [
    AppInstall {
        id: "snapmaker-orca",
        name: "Snapmaker Orca",
        folder: "Snapmaker_Orca",
        exe: "snapmaker-orca.exe",
    },
    AppInstall {
        id: "flash-studio",
        name: "Flash Studio Desktop",
        folder: "Flashforge/Flash Studio Desktop",
        exe: "flash studio.exe",
    },
    AppInstall {
        id: "elegoo-slicer",
        name: "ElegooSlicer",
        folder: "ElegooSlicer",
        exe: "elegoo-slicer.exe",
    },
    AppInstall {
        id: "qidi-studio",
        name: "QIDIStudio",
        folder: "QIDIStudio",
        exe: "QIDIStudio.exe",
    },
    AppInstall {
        id: "creality-print",
        name: "Creality Print",
        folder: "Creality/Creality Print 7.0",
        exe: "CrealityPrint.exe",
    },
    AppInstall {
        id: "creality-print",
        name: "Creality Print",
        folder: "Creality/Creality Print",
        exe: "CrealityPrint.exe",
    },
    AppInstall {
        id: "anycubic-slicer-next",
        name: "Anycubic Slicer Next",
        folder: "AnycubicSlicerNext",
        exe: "AnycubicSlicerNext.exe",
    },
    AppInstall {
        id: "anycubic-slicer-next",
        name: "Anycubic Slicer Next",
        folder: "Anycubic Slicer Next",
        exe: "AnycubicSlicerNext.exe",
    },
    AppInstall {
        id: "orca-slicer",
        name: "OrcaSlicer",
        folder: "OrcaSlicer",
        exe: "orca-slicer.exe",
    },
];

static DARWIN_APPS: [AppInstall; 9] = [
    AppInstall {
        id: "snapmaker-orca",
        name: "Snapmaker Orca",
        folder: "Snapmaker Orca",
        exe: "",
    },
    AppInstall {
        id: "flash-studio",
        name: "Flash Studio",
        folder: "Flash Studio",
        exe: "",
    },
    AppInstall {
        id: "flash-studio",
        name: "Flash Studio Desktop",
        folder: "Flash Studio Desktop",
        exe: "",
    },
    AppInstall {
        id: "elegoo-slicer",
        name: "ElegooSlicer",
        folder: "ElegooSlicer",
        exe: "",
    },
    AppInstall {
        id: "qidi-studio",
        name: "QIDIStudio",
        folder: "QIDIStudio",
        exe: "",
    },
    AppInstall {
        id: "creality-print",
        name: "Creality Print",
        folder: "Creality Print",
        exe: "",
    },
    AppInstall {
        id: "anycubic-slicer-next",
        name: "Anycubic Slicer Next",
        folder: "AnycubicSlicerNext",
        exe: "",
    },
    AppInstall {
        id: "anycubic-slicer-next",
        name: "Anycubic Slicer Next",
        folder: "Anycubic Slicer Next",
        exe: "",
    },
    AppInstall {
        id: "orca-slicer",
        name: "OrcaSlicer",
        folder: "OrcaSlicer",
        exe: "",
    },
];

pub(crate) fn current_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "other"
    }
}

fn family_apps(platform: &str) -> &'static [AppInstall] {
    match platform {
        "windows" => &WINDOWS_APPS[..],
        "darwin" => &DARWIN_APPS[..],
        _ => &[][..],
    }
}

fn present(path: Option<&Path>) -> Option<&Path> {
    path.filter(|p| !p.as_os_str().is_empty())
}

/// Install bases in priority order. Empty or absent system folders are dropped so
/// they never become relative paths. Mirrors options.go::installBases.
fn install_bases(
    platform: &str,
    program_files: Option<&Path>,
    program_files_x86: Option<&Path>,
    local_app_data: Option<&Path>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    match platform {
        "windows" => {
            bases.extend(present(program_files).map(Path::to_path_buf));
            bases.extend(present(program_files_x86).map(Path::to_path_buf));
            bases.extend(present(local_app_data).map(|local| local.join("Programs")));
        }
        "darwin" => {
            bases.push(PathBuf::from("/Applications"));
            bases.extend(present(home).map(|home| home.join("Applications")));
        }
        _ => {}
    }
    bases
}

fn install_location(platform: &str, base: &Path, app: &AppInstall) -> Location {
    if platform == "windows" {
        let dir = app
            .folder
            .split('/')
            .fold(base.to_path_buf(), |path, part| path.join(part));
        Location {
            id: app.id,
            name: app.name,
            root: dir.join("resources").join("profiles"),
            executable: dir.join(app.exe),
        }
    } else {
        let bundle = base.join(format!("{}.app", app.folder));
        Location {
            id: app.id,
            name: app.name,
            root: bundle.join("Contents").join("Resources").join("profiles"),
            executable: bundle,
        }
    }
}

fn push_unique(list: &mut Vec<Location>, location: Location) {
    if location.root.as_os_str().is_empty() || list.iter().any(|item| item.root == location.root) {
        return;
    }
    list.push(location);
}

/// Pure inventory of trusted Orca-family locations, in priority order. Explicit
/// overrides come first with no executable. Nothing is read from disk.
pub(crate) fn family_locations(
    platform: &str,
    program_files: Option<&Path>,
    program_files_x86: Option<&Path>,
    local_app_data: Option<&Path>,
    home: Option<&Path>,
    overrides: &[PathBuf],
) -> Vec<Location> {
    let mut result = Vec::new();
    for root in overrides.iter().filter(|path| !path.as_os_str().is_empty()) {
        push_unique(
            &mut result,
            Location {
                id: "system",
                name: "Custom local profile",
                root: root.clone(),
                executable: PathBuf::new(),
            },
        );
    }
    let bases = install_bases(
        platform,
        program_files,
        program_files_x86,
        local_app_data,
        home,
    );
    for app in family_apps(platform) {
        for base in &bases {
            push_unique(&mut result, install_location(platform, base, app));
        }
    }
    result
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Reads the live environment for `family_locations`, matching
/// options.go::installedOrcaFamilyLocations. The Windows Program Files default
/// applies only on Windows.
pub(crate) fn installed_family_locations() -> Vec<Location> {
    let platform = current_platform();
    let program_files = env_path("ProgramFiles")
        .or_else(|| (platform == "windows").then(|| PathBuf::from(r"C:\Program Files")));
    let overrides: Vec<PathBuf> = ["u1", "c5", "cc2", "qidi"]
        .iter()
        .filter_map(|target| {
            env_path(&format!(
                "BRIDGE_SLICER_PROFILE_{}",
                target.to_ascii_uppercase()
            ))
        })
        .collect();
    family_locations(
        platform,
        program_files.as_deref(),
        env_path("ProgramFiles(x86)").as_deref(),
        env_path("LOCALAPPDATA").as_deref(),
        env_path("HOME").as_deref(),
        &overrides,
    )
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// True when app_dir and every folder down to root are real directories. Any
/// link on that chain could redirect the scan, so it fails closed.
/// Mirrors orca.go::orcaChainIsReal.
fn chain_is_real(app_dir: &Path, root: &Path) -> bool {
    let Ok(relative) = root.strip_prefix(app_dir) else {
        return false;
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return false;
    }
    let mut current = app_dir.to_path_buf();
    if !is_real_dir(&current) {
        return false;
    }
    for part in relative.components() {
        current.push(part.as_os_str());
        if !is_real_dir(&current) {
            return false;
        }
    }
    true
}

fn app_is_real(location: &Location, platform: &str) -> bool {
    let app_dir = if platform == "windows" {
        location.executable.parent()
    } else {
        Some(location.executable.as_path())
    };
    app_dir.is_some_and(|dir| chain_is_real(dir, &location.root))
}

/// Keeps locations whose application folder chain is real. Explicit overrides
/// carry no executable and are used as given. Mirrors installedOrcaLocations.
pub(crate) fn installed_locations(platform: &str, locations: Vec<Location>) -> Vec<Location> {
    locations
        .into_iter()
        .filter(|location| {
            location.executable.as_os_str().is_empty() || app_is_real(location, platform)
        })
        .collect()
}

fn io_result<T>(result: io::Result<T>) -> Result<T, String> {
    result.map_err(|_| IO_ERROR.to_string())
}

fn sorted_entries(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

/// Mirrors orca.go::orcaVendorOwnedByApp. Branded forks contribute only their own
/// manufacturer. Generic OrcaSlicer and explicit roots keep multi-vendor scope.
fn vendor_owned_by_app(app_id: &str, vendor: &str) -> bool {
    let owned_by = |owner: &str| vendor.eq_ignore_ascii_case(owner);
    match app_id {
        "snapmaker-orca" => owned_by("Snapmaker"),
        "flash-studio" => owned_by("Flashforge"),
        "elegoo-slicer" => owned_by("Elegoo"),
        "qidi-studio" => owned_by("QIDI") || owned_by("X 5 Series"),
        "creality-print" => owned_by("Creality"),
        "anycubic-slicer-next" => owned_by("Anycubic"),
        _ => true,
    }
}

fn has_json_ext(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 5 && bytes[bytes.len() - 5..].eq_ignore_ascii_case(b".json")
}

// Case-sensitive trim, exactly like strings.TrimSuffix(base, ".json") in Go.
fn strip_json(name: &str) -> &str {
    name.strip_suffix(".json").unwrap_or(name)
}

fn stem_of(file: &Path) -> String {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    strip_json(&name).to_string()
}

/// Parses one vendor preset. A leading UTF-8 BOM is removed. A top-level null is
/// an empty preset, and any other non-object value is invalid.
fn parse_vendor_object(bytes: &[u8]) -> Result<Map<String, Value>, String> {
    let content = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let text = String::from_utf8_lossy(content);
    match serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())? {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        _ => Err("not a JSON object".into()),
    }
}

// Machine, process, filament, by the first of these path components present.
fn kind_of(relative: &Path) -> Option<&'static str> {
    let parents: Vec<String> = relative
        .parent()?
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    ["machine", "process", "filament"]
        .into_iter()
        .find(|kind| parents.iter().any(|parent| parent.as_str() == *kind))
}

/// Bounded counterpart of orca.go::readOrcaCatalog for one vendor. Reads only
/// machine, process and filament JSON. Links are never followed, depth is capped,
/// and the file count is bounded. Files over the size cap are skipped.
fn read_catalog(root: &Path, vendor: &str) -> Result<Index, String> {
    let mut dirs = vec![root.join(vendor)];
    if vendor == "Flashforge" {
        dirs.push(root.join("OrcaFilamentLibrary"));
    }
    let mut index = Index::new();
    let mut visited = 0usize;
    for dir in dirs {
        // WalkDir root semantics: a missing folder is empty, and a link or a
        // non-folder root is not traversed.
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {
                walk_catalog(&dir, &dir, &mut index, &mut visited)?
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(IO_ERROR.into()),
        }
    }
    Ok(index)
}

fn walk_catalog(
    dir: &Path,
    base: &Path,
    index: &mut Index,
    visited: &mut usize,
) -> Result<(), String> {
    for entry in io_result(sorted_entries(dir))? {
        let path = entry.path();
        let file_type = io_result(entry.file_type())?;
        if file_type.is_symlink() {
            continue;
        }
        let relative = path.strip_prefix(base).map_err(|_| IO_ERROR.to_string())?;
        if file_type.is_dir() {
            if relative.components().count() > MAX_DEPTH {
                continue;
            }
            walk_catalog(&path, base, index, visited)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !has_json_ext(&name) {
            continue;
        }
        let Some(kind) = kind_of(relative) else {
            continue;
        };
        *visited += 1;
        if *visited > MAX_CATALOG_FILES {
            return Err(format!(
                "vendor profile tree exceeds {MAX_CATALOG_FILES} files"
            ));
        }
        let size = io_result(entry.metadata())?.len();
        if size > MAX_PROFILE_BYTES {
            continue;
        }
        let bytes = io_result(fs::read(&path))?;
        let settings =
            parse_vendor_object(&bytes).map_err(|_| "vendor JSON is invalid".to_string())?;
        if settings.is_empty() {
            continue;
        }
        let stem = strip_json(&name).to_string();
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
                    file: path.clone(),
                    stem: stem.clone(),
                    settings: settings.clone(),
                });
        }
    }
    Ok(())
}

fn first_string(raw: Option<&Value>) -> String {
    match raw {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Explicit compatible_printers of a preset or its nearest ancestor. A missing
/// list is never read as "all printers". Mirrors orca.go::orcaCompatibility.
fn compatibility(index: &Index, kind: &str, name: &str) -> Option<Vec<String>> {
    let mut current = name.to_string();
    for _ in 0..MAX_COMPATIBILITY_HOPS {
        let leaf = &oem::raw_preset(index, kind, &current).ok()?.settings;
        if let Some(raw) = leaf.get("compatible_printers") {
            let list = raw.as_array()?;
            return Some(
                list.iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
            );
        }
        match leaf.get("inherits").and_then(Value::as_str) {
            Some(parent) if !parent.is_empty() => current = parent.to_string(),
            _ => return None,
        }
    }
    None
}

fn filaments_by_machine(index: &Index) -> ByMachine {
    let mut result: ByMachine = BTreeMap::new();
    for key in index.keys() {
        let Some(name) = key.strip_prefix("filament:") else {
            continue;
        };
        let Some(machines) = compatibility(index, "filament", name) else {
            continue;
        };
        let Ok(flattened) = oem::flatten(index, "filament", name) else {
            continue;
        };
        let material = first_string(flattened.get("filament_type"));
        if material.is_empty() {
            continue;
        }
        for machine in machines {
            result
                .entry(machine)
                .or_default()
                .entry(material.clone())
                .or_default()
                .push(name.to_string());
        }
    }
    for by_material in result.values_mut() {
        for names in by_material.values_mut() {
            names.sort();
        }
    }
    result
}

fn processes_by_machine(index: &Index) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in index.keys() {
        let Some(name) = key.strip_prefix("process:") else {
            continue;
        };
        let Some(machines) = compatibility(index, "process", name) else {
            continue;
        };
        for machine in machines {
            result.entry(machine).or_default().push(name.to_string());
        }
    }
    for choices in result.values_mut() {
        choices.sort();
        choices.dedup();
    }
    result
}

/// Mirrors orca.go::orcaPreferredChoice. The machine's declared default wins,
/// then the first sorted compatible name. An empty list means no choice.
fn preferred_choice(choices: &[String], defaults: Option<&Value>) -> String {
    let Some(first) = choices.first() else {
        return String::new();
    };
    let listed: Vec<&str> = match defaults {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().unwrap_or(""))
            .collect(),
        Some(Value::String(text)) => vec![text.as_str()],
        _ => Vec::new(),
    };
    for item in listed {
        if item.is_empty() {
            continue;
        }
        if let Some(choice) = choices.iter().find(|choice| choice.as_str() == item) {
            return choice.clone();
        }
    }
    first.clone()
}

/// A process narrower than half the physical nozzle is never selectable.
fn too_narrow(process: &Map<String, Value>, diameter: f64) -> bool {
    let value = first_string(process.get("line_width"));
    matches!(value.trim().parse::<f64>(), Ok(width) if width > 0.0 && width < diameter * 0.5)
}

/// Mirrors orca.go::orcaCompatibleProcess. The preferred default wins when it is
/// compatible and usable. Otherwise the best explicitly compatible process is
/// chosen by layer height. Ties break by name. Nothing guessed or incompatible.
fn compatible_process(
    index: &Index,
    compatible: &[String],
    preferred: &str,
    machine: &str,
    nozzle: &str,
) -> Result<(String, Map<String, Value>), String> {
    let diameter: f64 = nozzle
        .parse()
        .map_err(|_| "invalid machine nozzle for process lookup".to_string())?;
    if diameter <= 0.0 {
        return Err("invalid machine nozzle for process lookup".into());
    }
    if !preferred.is_empty() && compatible.iter().any(|name| name.as_str() == preferred) {
        if let Ok(process) = oem::flatten(index, "process", preferred) {
            if !too_narrow(&process, diameter) {
                return Ok((preferred.to_string(), process));
            }
        }
    }
    let mut best_name = String::new();
    let mut best_process = Map::new();
    let mut best_score = f64::INFINITY;
    for name in compatible {
        if name.as_str() == preferred {
            continue;
        }
        let Ok(process) = oem::flatten(index, "process", name) else {
            continue;
        };
        if too_narrow(&process, diameter) {
            continue;
        }
        // Layer height is read untrimmed, exactly like the Go source.
        let Ok(height) = first_string(process.get("layer_height")).parse::<f64>() else {
            continue;
        };
        if height <= 0.0 || height > diameter * 0.8 {
            continue;
        }
        let mut score = (height - diameter * 0.5).abs();
        if !name.to_lowercase().contains("standard") {
            score += diameter;
        }
        if score < best_score
            || (score == best_score && (best_name.is_empty() || name.as_str() < best_name.as_str()))
        {
            best_name = name.clone();
            best_process = process;
            best_score = score;
        }
    }
    if best_name.is_empty() {
        return Err(format!(
            "no installed compatible process for {machine:?} (default {preferred:?} unusable)"
        ));
    }
    Ok((best_name, best_process))
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// One uniform, supported nozzle diameter per installed tool. The tool count is
/// the length of the installed array. Mirrors orca.go::orcaNozzle.
fn nozzle_entries(raw: Option<&Value>) -> Result<(String, usize), String> {
    let entries = raw
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if entries.is_empty() || entries.len() > MAX_TOOLS {
        return Err(format!("nozzle_diameter must list 1..{MAX_TOOLS} tools"));
    }
    let mut nozzle = String::new();
    for entry in entries {
        let value = match entry {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
        .ok_or("nozzle_diameter has a non-numeric entry")?;
        let Some(supported) = NOZZLES.into_iter().find(|candidate| {
            candidate
                .parse::<f64>()
                .is_ok_and(|reference| (reference - value).abs() < 0.000001)
        }) else {
            return Err(format!(
                "unsupported nozzle diameter {} mm",
                display_value(entry)
            ));
        };
        if !nozzle.is_empty() && supported != nozzle {
            return Err("mixed nozzle diameters are not supported".into());
        }
        nozzle = supported.to_string();
    }
    Ok((nozzle, entries.len()))
}

fn is_abstract(leaf: &Map<String, Value>) -> bool {
    // fmt.Sprint(value) == "false" in Go matches the string or boolean spelling.
    match leaf.get("instantiation") {
        Some(Value::Bool(false)) => true,
        Some(Value::String(text)) => text == "false",
        _ => false,
    }
}

/// Mirrors orca.go::orcaTargetID. The ID hashes the machine file path so it
/// stays unique per install without exposing that path.
pub(crate) fn target_id(vendor: &str, machine: &str, file: &Path) -> String {
    let digest = sha256(clean_path(file).to_string_lossy().as_bytes());
    let fingerprint: String = digest[..5]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "orca-{}-{fingerprint}",
        slug(&format!("{vendor} {machine}"), 40)
    )
}

// Per-rune Go unicode.ToLower. The first char of Rust's full lowercase mapping
// equals the simple mapping for every char, including U+0130.
fn simple_lower(ch: char) -> char {
    ch.to_lowercase().next().unwrap_or(ch)
}

fn slug(value: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut dash = true;
    for ch in value.chars().map(simple_lower) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let mut slug = out.trim_matches('-').to_string();
    if slug.len() > limit {
        slug = slug[..limit].trim_matches('-').to_string();
    }
    if slug.is_empty() {
        "printer".into()
    } else {
        slug
    }
}

/// Lexical clean like Go filepath.Clean: drops '.', resolves '..', and normalizes
/// separators through components.
fn clean_path(path: &Path) -> PathBuf {
    let mut cleaned = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                let last = cleaned.components().next_back();
                let normal = matches!(last, Some(Component::Normal(_)));
                let anchored = matches!(last, Some(Component::RootDir | Component::Prefix(_)));
                if normal {
                    cleaned.pop();
                } else if !anchored {
                    cleaned.push("..");
                }
            }
            other => cleaned.push(other.as_os_str()),
        }
    }
    cleaned
}

/// Matches ^orca-[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{10}$ without a regex dependency.
pub(crate) fn is_orca_target_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("orca-") else {
        return false;
    };
    let Some((slug, digest)) = rest.rsplit_once('-') else {
        return false;
    };
    digest.len() == 10
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

// SHA-256 (FIPS 180-4). Implemented locally so the crate gains no dependency.
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for index in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let choose = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(w[index]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let majority = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(majority);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, value) in state.iter_mut().zip(v) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn resolve_machine(
    index: &Index,
    by_machine: &ByMachine,
    by_process: &BTreeMap<String, Vec<String>>,
    loc: &Location,
    vendor: &str,
    file: &Path,
) -> Result<Candidate, MachineError> {
    let stem = stem_of(file);
    let bytes = fs::read(file).map_err(|_| rejected("machine file unreadable"))?;
    let leaf = parse_vendor_object(&bytes).map_err(|_| rejected("machine JSON is invalid"))?;
    if is_abstract(&leaf) {
        return Err(MachineError::Abstract);
    }
    let machine_name = match leaf.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => stem.clone(),
    };
    let machine = oem::flatten(index, "machine", &stem).map_err(MachineError::Rejected)?;
    let technology = first_string(machine.get("printer_technology"));
    if !technology.is_empty() && technology != "FFF" {
        return Err(rejected(format!(
            "printer technology {technology} is not supported"
        )));
    }
    let model = first_string(machine.get("printer_model"));
    if model.is_empty() {
        return Err(rejected("printer_model is missing"));
    }
    if oem::is_builtin_model(&model) {
        return Err(rejected(format!(
            "{model} is already offered by a built-in engine"
        )));
    }
    let (nozzle, tools) =
        nozzle_entries(machine.get("nozzle_diameter")).map_err(MachineError::Rejected)?;
    for key in ["machine_start_gcode", "machine_end_gcode"] {
        if machine.get(key).and_then(Value::as_str).is_none() {
            return Err(rejected(format!("{key} is missing")));
        }
    }
    let compatible = by_process
        .get(&machine_name)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let (process_name, process) = compatible_process(
        index,
        compatible,
        &first_string(machine.get("default_print_profile")),
        &machine_name,
        &nozzle,
    )
    .map_err(MachineError::Rejected)?;

    let mut materials: Vec<(&'static str, String, Map<String, Value>)> = Vec::new();
    for kind in MATERIALS {
        let choices = by_machine
            .get(&machine_name)
            .and_then(|kinds| kinds.get(kind))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let name = preferred_choice(choices, machine.get("default_filament_profile"));
        if name.is_empty() {
            if kind == "PLA" {
                return Err(rejected(
                    "no PLA filament preset lists this machine as compatible",
                ));
            }
            continue;
        }
        let filament = oem::flatten(index, "filament", &name).map_err(MachineError::Rejected)?;
        materials.push((kind, name, filament));
    }
    let (pla_name, pla_settings) = materials
        .iter()
        .find(|(kind, _, _)| *kind == "PLA")
        .map(|(_, name, settings)| (name.clone(), settings))
        .ok_or_else(|| rejected("no PLA filament preset lists this machine as compatible"))?;

    // Machine, then process, then PLA. Firmware and hardware keys belong to the
    // installed machine. Identity keys are set last.
    let mut project = Map::new();
    for layer in [&machine, &process, pla_settings] {
        for (key, value) in layer {
            project.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in &machine {
        if key.starts_with("machine_")
            || matches!(
                key.as_str(),
                "nozzle_diameter"
                    | "change_filament_gcode"
                    | "layer_change_gcode"
                    | "before_layer_change_gcode"
            )
        {
            project.insert(key.clone(), value.clone());
        }
    }
    project.insert("printer_model".into(), json!(model));
    project.insert("printer_settings_id".into(), json!(machine_name));
    project.insert("print_settings_id".into(), json!(process_name));
    project.insert("filament_settings_id".into(), json!([pla_name]));
    project.insert("filament_type".into(), json!(["PLA"]));
    project.insert("filament_colour".into(), json!(["#FFFFFF"]));

    let id = target_id(vendor, &machine_name, file);
    let material_names: Vec<String> = materials
        .iter()
        .map(|(kind, _, _)| kind.to_string())
        .collect();
    let filament_profiles: Map<String, Value> = materials
        .iter()
        .map(|(kind, name, _)| (kind.to_string(), json!(name)))
        .collect();
    let material_settings: Map<String, Value> = materials
        .iter()
        .map(|(kind, _, settings)| (kind.to_string(), Value::Object(settings.clone())))
        .collect();
    let bundle = json!({
        "target": id,
        "vendor": vendor,
        "slicer": loc.name,
        "slicer_id": loc.id,
        "root": loc.root.to_string_lossy().into_owned(),
        "nozzle": nozzle,
        "physical_tools": tools,
        "profile": {
            "name": id,
            "display_name": model,
            "printer_model_id": model,
            "filament_slots": tools,
            "filament_profiles": filament_profiles,
            "default_filament_profile": pla_name,
            "project_settings": project
        },
        "material_settings": material_settings
    });
    Ok(Candidate {
        target: OrcaTarget {
            id,
            title: machine_name.clone(),
            slicer: loc.name.to_string(),
            vendor: vendor.to_string(),
            machine: machine_name,
            model,
            nozzle,
            physical_tools: tools,
            materials: material_names,
        },
        bundle,
        root: loc.root.clone(),
    })
}

fn scan_vendor(loc: &Location, vendor: &str) -> (Vec<Candidate>, Vec<OrcaRejection>) {
    let machine_dir = loc.root.join(vendor).join("machine");
    // read_dir follows a linked folder, so the folder itself must be real.
    if !is_real_dir(&machine_dir) {
        return (Vec::new(), Vec::new());
    }
    let Ok(entries) = sorted_entries(&machine_dir) else {
        return (Vec::new(), Vec::new());
    };
    // Regular files only: a link could point outside the installed profiles.
    let files: Vec<PathBuf> = entries
        .iter()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && has_json_ext(&entry.file_name().to_string_lossy())
        })
        .map(|entry| machine_dir.join(entry.file_name()))
        .collect();
    if files.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if files.len() > MAX_MACHINE_FILES {
        return (
            Vec::new(),
            vec![OrcaRejection {
                vendor: vendor.to_string(),
                machine: String::new(),
                slicer: loc.name.to_string(),
                reason: "too many machine presets to scan".into(),
            }],
        );
    }
    let index = match read_catalog(&loc.root, vendor) {
        Ok(index) => index,
        Err(error) => {
            return (
                Vec::new(),
                vec![OrcaRejection {
                    vendor: vendor.to_string(),
                    machine: String::new(),
                    slicer: loc.name.to_string(),
                    reason: format!("vendor profiles unreadable: {error}"),
                }],
            )
        }
    };
    let by_machine = filaments_by_machine(&index);
    let by_process = processes_by_machine(&index);
    let mut candidates = Vec::new();
    let mut problems = Vec::new();
    for file in &files {
        match resolve_machine(&index, &by_machine, &by_process, loc, vendor, file) {
            Ok(candidate) => candidates.push(candidate),
            Err(MachineError::Abstract) => {}
            Err(MachineError::Rejected(reason)) => problems.push(OrcaRejection {
                vendor: vendor.to_string(),
                machine: stem_of(file),
                slicer: loc.name.to_string(),
                reason,
            }),
        }
    }
    (candidates, problems)
}

/// Walks the locations in priority order. A machine is offered once per vendor and
/// machine name, so an earlier vendor fork wins over generic OrcaSlicer.
fn scan(locations: &[Location]) -> (Vec<Candidate>, Vec<OrcaRejection>) {
    let mut candidates = Vec::new();
    let mut rejected_rows = Vec::new();
    let mut offered: BTreeSet<(String, String)> = BTreeSet::new();
    for loc in locations {
        let Ok(entries) = sorted_entries(&loc.root) else {
            continue;
        };
        let mut scanned = 0usize;
        for entry in entries {
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            let vendor = entry.file_name().to_string_lossy().into_owned();
            // Bambu stays source-only. OrcaFilamentLibrary is a shared catalog.
            if !is_dir
                || vendor == "BBL"
                || vendor == "OrcaFilamentLibrary"
                || !vendor_owned_by_app(loc.id, &vendor)
            {
                continue;
            }
            // Count eligible vendor folders only, never root metadata files.
            if scanned >= MAX_VENDORS {
                break;
            }
            scanned += 1;
            let (found, problems) = scan_vendor(loc, &vendor);
            for candidate in found {
                let key = (
                    candidate.target.vendor.clone(),
                    candidate.target.machine.clone(),
                );
                if offered.insert(key) {
                    candidates.push(candidate);
                }
            }
            rejected_rows.extend(problems);
        }
    }
    (candidates, rejected_rows)
}

/// Mirrors orca.go::discoverOrca over already-trusted locations. Targets sort by
/// vendor, machine and ID. Rejections keep scan order, capped at 50.
pub(crate) fn discover_locations(locations: &[Location]) -> OrcaDiscovery {
    let (candidates, mut rejected) = scan(locations);
    let mut targets: Vec<OrcaTarget> = candidates
        .into_iter()
        .map(|candidate| candidate.target)
        .collect();
    targets
        .sort_by_cached_key(|target| format!("{} {} {}", target.vendor, target.machine, target.id));
    rejected.truncate(MAX_REJECTIONS);
    OrcaDiscovery { targets, rejected }
}

/// Discovery over the live, real-chain-filtered installed locations.
pub(crate) fn discover() -> OrcaDiscovery {
    discover_locations(&installed_locations(
        current_platform(),
        installed_family_locations(),
    ))
}

/// IDs of every installed target that fully resolves right now.
pub(crate) fn installed_ids() -> Vec<String> {
    discover()
        .targets
        .into_iter()
        .map(|target| target.id)
        .collect()
}

fn find_candidate(locations: &[Location], id: &str) -> Result<Candidate, String> {
    // The ID is never a path. It matches only freshly discovered targets.
    if !is_orca_target_id(id) {
        return Err("invalid installed profile identifier".into());
    }
    scan(locations)
        .0
        .into_iter()
        .find(|candidate| candidate.target.id == id)
        .ok_or_else(|| {
            "installed profile is not available or no longer resolves completely".to_string()
        })
}

/// Re-resolves one dynamic target into the same bundle shape the batch engine
/// reads (target, slicer_id, nozzle, profile, material_settings).
pub(crate) fn resolve_bundle_at(locations: &[Location], id: &str) -> Result<Value, String> {
    find_candidate(locations, id).map(|candidate| candidate.bundle)
}

pub(crate) fn resolve_bundle(id: &str) -> Result<Value, String> {
    resolve_bundle_at(
        &installed_locations(current_platform(), installed_family_locations()),
        id,
    )
}

// Some QIDIStudio builds ship either fixed executable name. Only the same
// folder's two fixed names count, and only regular non-empty files qualify.
fn is_regular_nonempty(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn qidi_studio_executable(location: &Location) -> PathBuf {
    let is_qidi_exe = location
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        == Some("QIDIStudio.exe");
    if location.id != "qidi-studio" || !is_qidi_exe {
        return location.executable.clone();
    }
    if is_regular_nonempty(&location.executable) {
        return location.executable.clone();
    }
    let alternative = location.executable.with_file_name("qidi-studio.exe");
    if is_regular_nonempty(&alternative) {
        return alternative;
    }
    location.executable.clone()
}

/// Returns the trusted app that owns the exact installed profile root. Only the
/// owning location of the re-resolved target is considered. Custom profile-only
/// roots have no application, so they fail closed. Mirrors
/// orca.go::resolveOrcaExecutableAt.
pub(crate) fn resolve_executable_at(
    locations: &[Location],
    id: &str,
    platform: &str,
) -> Result<OrcaSlicerApp, String> {
    let candidate = find_candidate(locations, id)?;
    for location in locations {
        if location.root != candidate.root || location.name != candidate.target.slicer.as_str() {
            continue;
        }
        if location.executable.as_os_str().is_empty() {
            return Err("installed profile has no trusted slicer application to open".into());
        }
        let mut executable = location.executable.clone();
        let app_dir = match platform {
            "windows" => {
                let dir = executable
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or("installed slicer path is missing or contains a link")?;
                executable = qidi_studio_executable(location);
                dir
            }
            // open -a takes the .app bundle, not its inner binary.
            "darwin" => executable.clone(),
            other => return Err(format!("opening Orca profiles is unsupported on {other}")),
        };
        if !chain_is_real(&app_dir, &location.root) {
            return Err("installed slicer path is missing or contains a link".into());
        }
        let metadata = fs::symlink_metadata(&executable)
            .map_err(|_| format!("installed {} application is missing", location.name))?;
        if platform == "windows" && (!metadata.is_file() || metadata.len() == 0) {
            return Err(format!(
                "installed {} executable is not a regular non-empty file",
                location.name
            ));
        }
        if platform == "darwin" && !metadata.is_dir() {
            return Err(format!(
                "installed {} application is not a real .app directory",
                location.name
            ));
        }
        return Ok(OrcaSlicerApp {
            id: location.id,
            name: location.name,
            executable,
        });
    }
    Err("installed slicer application for profile is unavailable".into())
}

pub(crate) fn resolve_executable(id: &str) -> Result<OrcaSlicerApp, String> {
    let platform = current_platform();
    resolve_executable_at(
        &installed_locations(platform, installed_family_locations()),
        id,
        platform,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    // Disposable synthetic tree. Removed on drop. No real slicer data is read.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "slicebridge-orca-profiles-test-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    type Files = BTreeMap<String, Value>;

    const ACME_MACHINE: &str = "machine/Acme Mini 0.4.json";
    const ACME_PROCESS: &str = "process/0.20mm Acme Standard.json";
    const ACME_FILAMENT: &str = "filament/Acme PLA.json";

    fn acme_sample() -> Files {
        let mut files = Files::new();
        files.insert(
            ACME_MACHINE.into(),
            json!({
                "name": "Acme Mini 0.4 nozzle", "printer_model": "Acme Mini", "printer_technology": "FFF",
                "nozzle_diameter": ["0.4"], "machine_start_gcode": "G28", "machine_end_gcode": "M104 S0",
                "default_print_profile": "0.20mm Acme Standard", "default_filament_profile": ["Acme PLA"]
            }),
        );
        files.insert(
            ACME_PROCESS.into(),
            json!({
                "name": "0.20mm Acme Standard", "compatible_printers": ["Acme Mini 0.4 nozzle"],
                "layer_height": "0.2", "wall_loops": "3"
            }),
        );
        files.insert(
            ACME_FILAMENT.into(),
            json!({
                "name": "Acme PLA", "compatible_printers": ["Acme Mini 0.4 nozzle"],
                "filament_type": ["PLA"], "nozzle_temperature": ["210"], "filament_flow_ratio": ["0.98"]
            }),
        );
        files
    }

    fn set(files: &mut Files, rel: &str, key: &str, value: Value) {
        files.get_mut(rel).unwrap()[key] = value;
    }

    fn write_vendor(root: &Path, vendor: &str, files: &Files) {
        for (rel, body) in files {
            let path = root.join(vendor).join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, serde_json::to_vec(body).unwrap()).unwrap();
        }
    }

    fn tree(vendor: &str, files: &Files) -> TempRoot {
        let root = TempRoot::new();
        write_vendor(root.path(), vendor, files);
        root
    }

    fn loc(id: &'static str, name: &'static str, root: &Path) -> Location {
        Location {
            id,
            name,
            root: root.to_path_buf(),
            executable: PathBuf::new(),
        }
    }

    fn discover_from(root: &Path) -> OrcaDiscovery {
        discover_locations(&[loc("test", "Test Orca", root)])
    }

    #[test]
    fn sha256_matches_published_vectors() {
        let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn target_id_slug_and_fingerprint_follow_go_rules() {
        assert_eq!(slug("İstanbul", 40), "istanbul");
        assert_eq!(slug("Ünïcode Test", 40), "n-code-test");
        assert_eq!(slug("!!!", 40), "printer");
        assert_eq!(slug(&"a".repeat(50), 40), "a".repeat(40));
        let file = Path::new("/profiles/Acme/machine/Acme Mini 0.4.json");
        let id = target_id("Acme", "Acme Mini 0.4 nozzle", file);
        assert!(id.starts_with("orca-acme-acme-mini-0-4-nozzle-"));
        assert!(is_orca_target_id(&id));
        assert_eq!(
            target_id(
                "Acme",
                "Acme Mini 0.4 nozzle",
                Path::new("/profiles/./Acme//machine/Acme Mini 0.4.json")
            ),
            id
        );
        assert_ne!(
            target_id(
                "Acme",
                "Acme Mini 0.4 nozzle",
                Path::new("/other/Acme Mini 0.4.json")
            ),
            id
        );
    }

    #[test]
    fn structurally_valid_sample_becomes_selectable_target() {
        let root = tree("Acme", &acme_sample());
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
        assert!(discovery.rejected.is_empty());
        let target = &discovery.targets[0];
        assert!(is_orca_target_id(&target.id));
        assert_eq!(target.model, "Acme Mini");
        assert_eq!(target.nozzle, "0.4");
        assert_eq!(target.physical_tools, 1);
        assert_eq!(target.materials, vec!["PLA".to_string()]);
        let payload = format!("{discovery:?}");
        assert!(!payload.contains(&*root.path().to_string_lossy()));

        let locations = [loc("test", "Test Orca", root.path())];
        let bundle = resolve_bundle_at(&locations, &target.id).unwrap();
        assert_eq!(bundle["target"], json!(target.id));
        assert_eq!(bundle["slicer_id"], json!("test"));
        assert_eq!(bundle["nozzle"], json!("0.4"));
        assert_eq!(bundle["physical_tools"], json!(1));
        assert_eq!(bundle["profile"]["name"], json!(target.id));
        assert_eq!(bundle["profile"]["printer_model_id"], json!("Acme Mini"));
        assert_eq!(bundle["profile"]["filament_slots"], json!(1));
        assert_eq!(
            bundle["profile"]["filament_profiles"]["PLA"],
            json!("Acme PLA")
        );
        assert_eq!(
            bundle["profile"]["project_settings"]["printer_model"],
            json!("Acme Mini")
        );
        assert_eq!(
            bundle["profile"]["project_settings"]["print_settings_id"],
            json!("0.20mm Acme Standard")
        );
    }

    #[test]
    fn physical_tools_come_from_installed_nozzle_array() {
        let mut files = acme_sample();
        set(
            &mut files,
            ACME_MACHINE,
            "nozzle_diameter",
            json!(["0.4", "0.4"]),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
        assert_eq!(discovery.targets[0].physical_tools, 2);
    }

    #[test]
    fn unsupported_or_missing_profiles_are_rejected_with_reasons() {
        type Mutation = fn(&mut Files);
        let cases: [(&str, Mutation, &str); 8] = [
            (
                "default process file missing",
                |f: &mut Files| {
                    f.remove(ACME_PROCESS);
                },
                "no installed compatible process",
            ),
            (
                "process does not list machine",
                |f: &mut Files| set(f, ACME_PROCESS, "compatible_printers", json!(["Other"])),
                "no installed compatible process",
            ),
            (
                "no PLA lists machine",
                |f: &mut Files| set(f, ACME_FILAMENT, "compatible_printers", json!(["Other"])),
                "no PLA filament",
            ),
            (
                "mixed nozzle sizes",
                |f: &mut Files| set(f, ACME_MACHINE, "nozzle_diameter", json!(["0.4", "0.6"])),
                "mixed nozzle",
            ),
            (
                "unsupported nozzle size",
                |f: &mut Files| set(f, ACME_MACHINE, "nozzle_diameter", json!(["0.3"])),
                "unsupported nozzle diameter",
            ),
            (
                "resin machine",
                |f: &mut Files| set(f, ACME_MACHINE, "printer_technology", json!("SLA")),
                "printer technology SLA",
            ),
            (
                "start/end script missing",
                |f: &mut Files| {
                    f.get_mut(ACME_MACHINE)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("machine_end_gcode");
                },
                "machine_end_gcode is missing",
            ),
            (
                "model already served by built-in engine",
                |f: &mut Files| set(f, ACME_MACHINE, "printer_model", json!("X-Plus 5")),
                "already offered by a built-in engine",
            ),
        ];
        for (name, mutate, reason) in cases {
            let mut files = acme_sample();
            mutate(&mut files);
            let root = tree("Acme", &files);
            let discovery = discover_from(root.path());
            assert!(
                discovery.targets.is_empty(),
                "{name}: unsafe machine offered"
            );
            assert_eq!(
                discovery.rejected.len(),
                1,
                "{name}: {:?}",
                discovery.rejected
            );
            assert!(
                discovery.rejected[0].reason.contains(reason),
                "{name}: expected {reason:?}, got {:?}",
                discovery.rejected
            );
        }
    }

    #[test]
    fn bambu_vendor_is_never_offered_as_orca_target() {
        let root = tree("BBL", &acme_sample());
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert!(discovery.rejected.is_empty());
    }

    #[test]
    fn malformed_or_unknown_identifiers_fail_closed() {
        let root = tree("Acme", &acme_sample());
        let locations = [loc("test", "Test Orca", root.path())];
        for id in [
            "",
            "u1",
            "qidi",
            "../../etc/passwd",
            "orca-acme-mini-0-4-nozzle-XYZ",
            "orca-acme-012345678",
        ] {
            assert!(!is_orca_target_id(id), "{id:?} must not be valid");
            assert!(
                resolve_bundle_at(&locations, id).is_err(),
                "{id:?} resolved"
            );
        }
        assert!(resolve_bundle_at(&locations, "orca-acme-mini-0-4-nozzle-0123456789").is_err());
    }

    #[test]
    fn creator5_pro_recovers_all_four_nozzle_profiles_without_ad5m_default() {
        let mut files = Files::new();
        let nozzles = [
            (
                "0.25",
                "0.12",
                "0.12mm Standard @FF C5 0.25 nozzle",
                "0.12mm Standard @FF C5 0.25 nozzle",
            ),
            (
                "0.4",
                "0.20",
                "0.20mm Standard @AD5M Pro 0.4",
                "0.20mm Standard @FF C5",
            ),
            (
                "0.6",
                "0.30",
                "0.20mm Standard @AD5M Pro 0.4",
                "0.30mm Standard @FF C5 0.6 nozzle",
            ),
            (
                "0.8",
                "0.40",
                "0.20mm Standard @AD5M Pro 0.4",
                "0.40mm Standard @FF C5 0.8 nozzle",
            ),
        ];
        let mut expected = BTreeMap::new();
        for (diameter, layer, preferred, selected) in nozzles {
            let machine = format!("Flashforge Creator 5 Pro {diameter} nozzle");
            let filament = format!("Generic PLA @FF C5P {diameter}");
            files.insert(
                format!("machine/{machine}.json"),
                json!({
                    "name": machine, "printer_model": "Flashforge Creator 5 Pro", "printer_technology": "FFF",
                    "nozzle_diameter": [diameter, diameter, diameter, diameter],
                    "machine_start_gcode": "G28", "machine_end_gcode": "M104 S0",
                    "default_print_profile": preferred, "default_filament_profile": [filament.as_str()]
                }),
            );
            files.insert(
                format!("process/{selected}.json"),
                json!({"name": selected, "compatible_printers": [machine.as_str()], "layer_height": layer, "wall_loops": "3"}),
            );
            files.insert(
                format!("filament/{filament}.json"),
                json!({
                    "name": filament, "compatible_printers": [machine.as_str()], "filament_type": ["PLA"],
                    "nozzle_temperature": ["210"], "filament_flow_ratio": ["0.98"]
                }),
            );
            expected.insert(diameter, selected);
        }
        // An incompatible vendor default for another model must never be used.
        files.insert(
            "process/0.20mm Standard @AD5M Pro 0.4.json".into(),
            json!({"compatible_printers": ["Flashforge Adventurer 5M Pro 0.4"], "layer_height": "0.20", "wall_loops": "3"}),
        );
        let root = tree("Flashforge", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 4, "{discovery:?}");
        assert!(discovery.rejected.is_empty());
        let locations = [loc("flash", "Flash Studio", root.path())];
        for target in &discovery.targets {
            assert_eq!(target.model, "Flashforge Creator 5 Pro");
            assert_eq!(target.physical_tools, 4);
            assert_eq!(target.title, target.machine);
            let bundle = resolve_bundle_at(&locations, &target.id).unwrap();
            assert_eq!(bundle["nozzle"], json!(target.nozzle));
            let process = bundle["profile"]["project_settings"]["print_settings_id"]
                .as_str()
                .unwrap();
            assert_ne!(process, "0.20mm Standard @AD5M Pro 0.4");
            assert_eq!(process, expected[target.nozzle.as_str()]);
        }
    }

    #[test]
    fn fallback_requires_explicit_compatibility_and_sane_layer_height() {
        let mut files = acme_sample();
        set(
            &mut files,
            ACME_MACHINE,
            "default_print_profile",
            json!("Nonexistent vendor typo"),
        );
        files.insert(
            "process/0.20mm for Other.json".into(),
            json!({"compatible_printers": ["Other machine"], "layer_height": "0.2"}),
        );
        files.insert(
            "process/0.40mm Bad Layer.json".into(),
            json!({"compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.40"}),
        );
        files.insert(
            "process/0.20mm Standard Alternate.json".into(),
            json!({"compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.2"}),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
        assert!(discovery.rejected.is_empty());

        files.remove("process/0.20mm Standard Alternate.json");
        files.remove(ACME_PROCESS);
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert_eq!(discovery.rejected.len(), 1);
        assert!(discovery.rejected[0]
            .reason
            .contains("no installed compatible process"));
    }

    #[test]
    fn process_selection_is_deterministic_without_default() {
        let mut files = acme_sample();
        files
            .get_mut(ACME_MACHINE)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("default_print_profile");
        files.insert(
            "process/0.32mm Draft.json".into(),
            json!({"compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.32"}),
        );
        files.insert(
            "process/0.18mm Standard.json".into(),
            json!({"compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.18"}),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1, "{discovery:?}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_vendor_and_machine_files_are_ignored() {
        let outside = tree("Acme", &acme_sample());
        let linked = TempRoot::new();
        std::os::unix::fs::symlink(outside.path().join("Acme"), linked.path().join("Acme"))
            .unwrap();
        let discovery = discover_from(linked.path());
        assert!(discovery.targets.is_empty());
        assert!(discovery.rejected.is_empty());

        let mut files = acme_sample();
        files.remove(ACME_MACHINE);
        let root = tree("Acme", &files);
        let machine_dir = root.path().join("Acme").join("machine");
        fs::create_dir_all(&machine_dir).unwrap();
        let elsewhere = TempRoot::new();
        let outside_file = elsewhere.path().join("machine.json");
        fs::write(
            &outside_file,
            serde_json::to_vec(&acme_sample()[ACME_MACHINE]).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside_file, machine_dir.join("Acme Mini 0.4.json")).unwrap();
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert!(discovery.rejected.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_machine_folder_is_ignored() {
        let outside = tree("Acme", &acme_sample());
        let mut files = acme_sample();
        files.retain(|rel, _| !rel.starts_with("machine/"));
        let root = tree("Acme", &files);
        std::os::unix::fs::symlink(
            outside.path().join("Acme").join("machine"),
            root.path().join("Acme").join("machine"),
        )
        .unwrap();
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert!(discovery.rejected.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn app_chain_rejects_symlinked_application_folder() {
        let base = TempRoot::new();
        let bundle = base.path().join("real").join("Snapmaker Orca.app");
        let profiles = bundle.join("Contents").join("Resources").join("profiles");
        fs::create_dir_all(&profiles).unwrap();
        assert!(chain_is_real(&bundle, &profiles));
        let missing = base.path().join("missing.app");
        assert!(!chain_is_real(
            &missing,
            &missing.join("Contents").join("Resources").join("profiles")
        ));
        let link = base.path().join("linked.app");
        std::os::unix::fs::symlink(&bundle, &link).unwrap();
        assert!(!chain_is_real(
            &link,
            &link.join("Contents").join("Resources").join("profiles")
        ));
    }

    #[test]
    fn vendor_fork_copy_wins_over_generic_orca_and_broken_copy_falls_back() {
        let fork = tree("Snapmaker", &acme_sample());
        let generic = tree("Snapmaker", &acme_sample());
        let discovery = discover_locations(&[
            loc("snapmaker-orca", "Snapmaker Orca", fork.path()),
            loc("orca-slicer", "OrcaSlicer", generic.path()),
        ]);
        assert_eq!(discovery.targets.len(), 1);
        assert_eq!(discovery.targets[0].slicer, "Snapmaker Orca");
        assert!(discovery.rejected.is_empty());

        let mut broken = acme_sample();
        broken.remove(ACME_FILAMENT);
        let broken_root = tree("Snapmaker", &broken);
        let discovery = discover_locations(&[
            loc("snapmaker-orca", "Snapmaker Orca", broken_root.path()),
            loc("orca-slicer", "OrcaSlicer", generic.path()),
        ]);
        assert_eq!(discovery.targets.len(), 1);
        assert_eq!(discovery.targets[0].slicer, "OrcaSlicer");
        assert_eq!(discovery.rejected.len(), 1);
        assert_eq!(discovery.rejected[0].slicer, "Snapmaker Orca");
        assert!(discovery.rejected[0].reason.contains("no PLA filament"));
    }

    #[test]
    fn branded_forks_only_contribute_their_own_manufacturer() {
        let fork = TempRoot::new();
        write_vendor(fork.path(), "Snapmaker", &acme_sample());
        write_vendor(fork.path(), "Anker", &acme_sample());
        let generic = tree("Anker", &acme_sample());
        let locations = [
            loc("snapmaker-orca", "Snapmaker Orca", fork.path()),
            loc("orca-slicer", "OrcaSlicer", generic.path()),
        ];
        let discovery = discover_locations(&locations);
        assert_eq!(discovery.targets.len(), 2, "{discovery:?}");
        assert!(discovery.rejected.is_empty());
        let owners: BTreeMap<&str, &str> = discovery
            .targets
            .iter()
            .map(|target| (target.vendor.as_str(), target.slicer.as_str()))
            .collect();
        assert_eq!(owners["Snapmaker"], "Snapmaker Orca");
        assert_eq!(owners["Anker"], "OrcaSlicer");
        let foreign = target_id(
            "Anker",
            "Acme Mini 0.4 nozzle",
            &fork.path().join("Anker").join(ACME_MACHINE),
        );
        assert!(resolve_bundle_at(&locations, &foreign).is_err());
    }

    #[test]
    fn vendor_ownership_whitelist_does_not_filter_generic_or_explicit_profiles() {
        for (app, vendor, allowed) in [
            ("snapmaker-orca", "Snapmaker", true),
            ("snapmaker-orca", "Anker", false),
            ("elegoo-slicer", "ELEGOO", true),
            ("elegoo-slicer", "Flashforge", false),
            ("flash-studio", "Flashforge", true),
            ("flash-studio", "Anker", false),
            ("qidi-studio", "QIDI", true),
            ("qidi-studio", "X 5 Series", true),
            ("qidi-studio", "Anker", false),
            ("creality-print", "Creality", true),
            ("creality-print", "Anycubic", false),
            ("anycubic-slicer-next", "Anycubic", true),
            ("anycubic-slicer-next", "Creality", false),
            ("orca-slicer", "Anker", true),
            ("system", "Anker", true),
        ] {
            assert_eq!(vendor_owned_by_app(app, vendor), allowed, "{app} {vendor}");
        }
    }

    #[test]
    fn abstract_machine_presets_are_skipped_silently() {
        let mut files = acme_sample();
        files.insert(
            "machine/fdm_acme_common.json".into(),
            json!({"name": "fdm_acme_common", "instantiation": "false", "nozzle_diameter": ["0.4"]}),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
        assert!(discovery.rejected.is_empty());
    }

    #[test]
    fn process_files_beyond_scan_depth_are_not_indexed() {
        let mut files = acme_sample();
        files.remove(ACME_PROCESS);
        files.insert(
            "process/a/b/c/d/0.20mm Acme Standard.json".into(),
            json!({"name": "0.20mm Acme Standard", "compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.2"}),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert_eq!(discovery.rejected.len(), 1);
        assert!(discovery.rejected[0]
            .reason
            .contains("no installed compatible process"));
    }

    #[test]
    fn windows_install_tree_discovers_vendor_first_without_duplicates() {
        let pf = TempRoot::new();
        let local = TempRoot::new();
        write_vendor(
            &pf.path()
                .join("Snapmaker_Orca")
                .join("resources")
                .join("profiles"),
            "Snapmaker",
            &acme_sample(),
        );
        let generic = local
            .path()
            .join("Programs")
            .join("OrcaSlicer")
            .join("resources")
            .join("profiles");
        write_vendor(&generic, "Snapmaker", &acme_sample());
        write_vendor(&generic, "Zed", &acme_sample());
        let locations = family_locations(
            "windows",
            Some(pf.path()),
            None,
            Some(local.path()),
            None,
            &[],
        );
        let discovery = discover_locations(&locations);
        assert_eq!(discovery.targets.len(), 2, "{discovery:?}");
        assert!(discovery.rejected.is_empty());
        for target in &discovery.targets {
            match target.vendor.as_str() {
                "Snapmaker" => assert_eq!(target.slicer, "Snapmaker Orca"),
                "Zed" => assert_eq!(target.slicer, "OrcaSlicer"),
                other => panic!("unexpected vendor {other}"),
            }
        }
    }

    #[test]
    fn mac_install_tree_discovers_user_applications() {
        let home = TempRoot::new();
        let profiles = home
            .path()
            .join("Applications")
            .join("OrcaSlicer.app")
            .join("Contents")
            .join("Resources")
            .join("profiles");
        write_vendor(&profiles, "Acme", &acme_sample());
        let mine: Vec<Location> =
            family_locations("darwin", None, None, None, Some(home.path()), &[])
                .into_iter()
                .filter(|location| location.root.starts_with(home.path()))
                .collect();
        let discovery = discover_locations(&mine);
        assert_eq!(discovery.targets.len(), 1, "{discovery:?}");
        assert_eq!(discovery.targets[0].slicer, "OrcaSlicer");
    }

    #[test]
    fn non_vendor_root_files_do_not_exhaust_vendor_scan_cap() {
        let root = tree("Acme", &acme_sample());
        for index in 0..(MAX_VENDORS + 15) {
            fs::write(root.path().join(format!("000-index-{index:03}.json")), "{}").unwrap();
        }
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
    }

    #[test]
    fn abstract_process_does_not_shadow_physical_nozzle_process() {
        for (nozzle, layer, width) in [("0.6", "0.30", "0.6"), ("0.8", "0.40", "0.82")] {
            let mut files = acme_sample();
            files.remove(ACME_MACHINE);
            let machine = format!("Acme Mini {nozzle} nozzle");
            files.insert(
                format!("machine/{machine}.json"),
                json!({
                    "name": machine, "printer_model": "Acme Mini", "printer_technology": "FFF",
                    "nozzle_diameter": [nozzle], "machine_start_gcode": "G28", "machine_end_gcode": "M104 S0",
                    "default_print_profile": "abstract 0.2mm", "default_filament_profile": ["Acme PLA"]
                }),
            );
            set(
                &mut files,
                ACME_FILAMENT,
                "compatible_printers",
                json!([machine.as_str()]),
            );
            files.insert(
                "process/abstract 0.2mm.json".into(),
                json!({"name": "abstract 0.2mm", "instantiation": "false", "compatible_printers": [machine.as_str()], "layer_height": "0.2", "line_width": "0.22"}),
            );
            let selected = format!("Acme {layer}mm Standard {nozzle}");
            files.insert(
                format!("process/{selected}.json"),
                json!({"name": selected, "instantiation": "true", "compatible_printers": [machine.as_str()], "layer_height": layer, "line_width": width}),
            );
            let root = tree("Acme", &files);
            let discovery = discover_from(root.path());
            assert_eq!(discovery.targets.len(), 1, "{nozzle}: {discovery:?}");
            assert!(discovery.rejected.is_empty());
            let locations = [loc("test", "Test Orca", root.path())];
            let bundle = resolve_bundle_at(&locations, &discovery.targets[0].id).unwrap();
            let settings = &bundle["profile"]["project_settings"];
            assert_eq!(settings["print_settings_id"], json!(selected));
            assert_eq!(settings["line_width"], json!(width));
            assert_eq!(bundle["nozzle"], json!(nozzle));
            assert_eq!(settings["nozzle_diameter"], json!([nozzle]));
        }
    }

    #[test]
    fn too_narrow_only_process_is_not_selectable() {
        let mut files = acme_sample();
        set(&mut files, ACME_MACHINE, "nozzle_diameter", json!(["0.6"]));
        set(&mut files, ACME_PROCESS, "line_width", json!("0.22"));
        set(&mut files, ACME_PROCESS, "instantiation", json!("false"));
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert!(discovery.targets.is_empty());
        assert_eq!(discovery.rejected.len(), 1);
        assert!(discovery.rejected[0]
            .reason
            .contains("no installed compatible process"));
    }

    #[test]
    fn already_compatible_preferred_process_is_unchanged() {
        let mut files = acme_sample();
        set(&mut files, ACME_PROCESS, "line_width", json!("0.22"));
        set(&mut files, ACME_PROCESS, "instantiation", json!("false"));
        files.insert(
            "process/0.20mm Alternative.json".into(),
            json!({"name": "0.20mm Alternative", "instantiation": "true", "compatible_printers": ["Acme Mini 0.4 nozzle"], "layer_height": "0.2", "line_width": "0.42"}),
        );
        let root = tree("Acme", &files);
        let discovery = discover_from(root.path());
        assert_eq!(discovery.targets.len(), 1);
        let locations = [loc("test", "Test Orca", root.path())];
        let bundle = resolve_bundle_at(&locations, &discovery.targets[0].id).unwrap();
        assert_eq!(
            bundle["profile"]["project_settings"]["print_settings_id"],
            json!("0.20mm Acme Standard")
        );
        assert_eq!(
            bundle["profile"]["project_settings"]["line_width"],
            json!("0.22")
        );
    }

    #[test]
    fn executable_resolution_requires_real_app_chain_and_known_platform() {
        let base = TempRoot::new();
        let bundle = base.path().join("OrcaSlicer.app");
        let profiles = bundle.join("Contents").join("Resources").join("profiles");
        write_vendor(&profiles, "Acme", &acme_sample());
        let location = Location {
            id: "orca-slicer",
            name: "OrcaSlicer",
            root: profiles.clone(),
            executable: bundle.clone(),
        };
        let discovery = discover_locations(std::slice::from_ref(&location));
        let id = discovery.targets[0].id.clone();
        let app = resolve_executable_at(std::slice::from_ref(&location), &id, "darwin").unwrap();
        assert_eq!(app.executable, bundle);
        assert_eq!(app.name, "OrcaSlicer");
        assert!(resolve_executable_at(std::slice::from_ref(&location), &id, "other").is_err());
        let custom = Location {
            id: "system",
            name: "Custom local profile",
            root: profiles,
            executable: PathBuf::new(),
        };
        assert!(resolve_executable_at(std::slice::from_ref(&custom), &id, "darwin").is_err());
    }
}
