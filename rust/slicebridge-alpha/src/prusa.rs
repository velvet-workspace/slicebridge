//! Read-only discovery of installed PrusaSlicer printer presets, ported from
//! `native/presets/prusa.go`. A user printer becomes a target only when it
//! resolves completely: an FFF printer with a bed shape, exactly one supported
//! nozzle, and a compatible print and PLA filament found in the user folders or
//! the vendor bundles. Compatibility follows PrusaSlicer. A non-empty
//! `compatible_printers` list decides alone. Otherwise a non-empty
//! `compatible_printers_condition` decides. A preset with neither applies to every
//! printer. Parents come only from a preset's own `inherits` list.
//!
//! Every target is reported with `ready: false`. Per-printer PrusaSlicer export
//! has not been verified, so listing a target never authorises conversion.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use crate::prusa_condition::evaluate_prusa_condition;
use crate::prusa_ini::{
    parse_prusa_ini, split_prusa_list, split_prusa_values, PrusaIni, PrusaSection,
};

const MAX_PRUSA_REJECTIONS: usize = 50;
const MAX_PRUSA_FILES_PER_DIR: usize = 512;
const MAX_PRUSA_VENDOR_FILES: usize = 256;
const MAX_PRUSA_FILE_BYTES: usize = 8 << 20;
const MAX_PRUSA_TOTAL_BYTES: usize = 128 << 20;
const MAX_PRUSA_INHERIT: usize = 8;
// Bound total parent visits as well as depth to prevent a wide inheritance tree
// from expanding exponentially. Exceeding this limit rejects only that preset.
const MAX_PRUSA_INHERIT_VISITS: usize = 4096;
const PRUSA_SLICER_LABEL: &str = "PrusaSlicer";
const PRUSA_CUSTOM_VENDOR: &str = "Custom";
const PRUSA_UNREADABLE: &str = "profile file cannot be read safely";
const PRUSA_EXPORT_NOT_VERIFIED: &str =
    "PrusaSlicer per-printer export is not verified yet. Use the generic PrusaSlicer project.";
const PRUSA_NOZZLES: [&str; 5] = ["0.2", "0.25", "0.4", "0.6", "0.8"];
const PRUSA_KINDS: [&str; 3] = ["printer", "print", "filament"];

/// One installed user printer preset that resolved completely. `print` and
/// `filament` are the internal preset names a later conversion would need. They
/// are not for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrusaTarget {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) slicer: &'static str,
    pub(crate) source_root: &'static str,
    pub(crate) vendor: String,
    pub(crate) model: String,
    pub(crate) nozzle: &'static str,
    pub(crate) print: String,
    pub(crate) filament: String,
    pub(crate) ready: bool,
    pub(crate) reason: &'static str,
}

/// Explains why an installed printer preset is not offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrusaRejection {
    pub(crate) vendor: String,
    pub(crate) machine: String,
    pub(crate) slicer: &'static str,
    pub(crate) reason: String,
}

/// Installed Prusa printers: complete targets plus bounded rejection reasons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrusaDiscovery {
    pub(crate) targets: Vec<PrusaTarget>,
    pub(crate) rejected: Vec<PrusaRejection>,
}

/// Trusted roots. `user` holds the printer/, print/ and filament/ folders.
/// `vendor` holds the bundled vendor .ini files. `None` means no such root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PrusaLocations {
    user: Option<PathBuf>,
    vendor: Option<PathBuf>,
}

/// The [presets] block of PrusaSlicer.ini. It only ranks candidates and never adds one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PrusaSelection {
    printer: String,
    print: String,
    filament: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrusaProfile {
    file: PathBuf,
    values: PrusaSection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrusaPrinterSource {
    file: PathBuf,
    name: String,
}

/// Built once per discovery and read-only afterwards.
#[derive(Debug, Default)]
struct PrusaIndex {
    // "kind:name" -> one entry per defining file
    profiles: BTreeMap<String, Vec<PrusaProfile>>,
    // vendor file -> [vendor] name
    vendors: BTreeMap<PathBuf, String>,
    // kind -> sorted selectable preset names
    names: BTreeMap<String, Vec<String>>,
    // user printer presets, in file order
    printers: Vec<PrusaPrinterSource>,
    selected: PrusaSelection,
}

/// One compatible preset that is ranked against its alternatives.
#[derive(Debug, PartialEq)]
struct PrusaChoice {
    name: String,
    selected: bool,
    foreign: bool,
    score: f64,
}

#[derive(Debug, PartialEq, Eq)]
enum PrusaScanError {
    Missing,
    Failed,
}

/// Discovers installed PrusaSlicer printers from the fixed install locations.
pub(crate) fn discover_prusa() -> PrusaDiscovery {
    discover_prusa_at(&installed_prusa_locations())
}

/// Lists the IDs of installed targets, in the same order as `discover_prusa`.
pub(crate) fn installed_prusa_target_ids() -> Vec<String> {
    discover_prusa()
        .targets
        .into_iter()
        .map(|target| target.id)
        .collect()
}

/// Re-resolves an ID from disk. The ID is compared only with freshly discovered
/// targets and is never treated as a path.
pub(crate) fn resolve_prusa_target(id: &str) -> Result<PrusaTarget, String> {
    resolve_prusa_target_at(&installed_prusa_locations(), id)
}

/// Reports whether an ID has the shape this module produces:
/// `prusa-<slug>-<10 lowercase hex>`.
pub(crate) fn is_prusa_target_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("prusa-") else {
        return false;
    };
    let Some((slug, hash)) = rest.rsplit_once('-') else {
        return false;
    };
    let hash_ok = hash.len() == 10
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let slug_ok = !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    hash_ok && slug_ok
}

fn resolve_prusa_target_at(loc: &PrusaLocations, id: &str) -> Result<PrusaTarget, String> {
    if !is_prusa_target_id(id) {
        return Err("invalid installed printer identifier".to_owned());
    }
    discover_prusa_at(loc)
        .targets
        .into_iter()
        .find(|target| target.id == id)
        .ok_or_else(|| {
            "installed printer profile is not available or no longer resolves completely".to_owned()
        })
}

fn installed_prusa_locations() -> PrusaLocations {
    if cfg!(windows) {
        let user = env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(|value| PathBuf::from(value).join("PrusaSlicer"));
        let program_files = env::var_os("ProgramFiles")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let vendor = program_files
            .join("Prusa3D")
            .join("PrusaSlicer")
            .join("resources")
            .join("profiles");
        return PrusaLocations {
            user,
            vendor: Some(vendor),
        };
    }
    if cfg!(target_os = "macos") {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let system = PathBuf::from("/Applications/PrusaSlicer.app/Contents/Resources/profiles");
        let per_user = home.as_ref().map(|home| {
            home.join("Applications")
                .join("PrusaSlicer.app")
                .join("Contents")
                .join("Resources")
                .join("profiles")
        });
        let vendor = if !system.exists() && per_user.is_some() {
            per_user
        } else {
            Some(system)
        };
        let user = home.map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("PrusaSlicer")
        });
        return PrusaLocations { user, vendor };
    }
    PrusaLocations::default()
}

fn discover_prusa_at(loc: &PrusaLocations) -> PrusaDiscovery {
    let (index, mut problems) = load_prusa_index(loc);
    let mut targets = Vec::new();
    for source in &index.printers {
        match index.resolve_printer(source) {
            Ok(target) => targets.push(target),
            Err(reason) => problems.push(PrusaRejection {
                vendor: PRUSA_CUSTOM_VENDOR.to_owned(),
                machine: source.name.clone(),
                slicer: PRUSA_SLICER_LABEL,
                reason,
            }),
        }
    }
    targets.sort_by_cached_key(target_sort_key);
    problems.truncate(MAX_PRUSA_REJECTIONS);
    PrusaDiscovery {
        targets,
        rejected: problems,
    }
}

fn target_sort_key(target: &PrusaTarget) -> String {
    format!("{} {} {}", target.vendor, target.name, target.id)
}

/// Reads vendor bundles first and then user folders. A file that cannot be read
/// or parsed is reported and skipped, so one bad file never hides the others.
fn load_prusa_index(loc: &PrusaLocations) -> (PrusaIndex, Vec<PrusaRejection>) {
    let mut index = PrusaIndex::default();
    let mut problems = Vec::new();
    let mut budget = MAX_PRUSA_TOTAL_BYTES;
    if let Some(vendor) = &loc.vendor {
        match list_prusa_files(vendor, MAX_PRUSA_VENDOR_FILES) {
            Ok(files) => {
                load_prusa_files(&mut index, &files, false, &mut budget, &mut problems);
            }
            Err(PrusaScanError::Missing) => {}
            Err(PrusaScanError::Failed) => problems.push(load_problem(
                "vendor bundles",
                "vendor folder cannot be scanned",
            )),
        }
    }
    if let Some(user) = &loc.user {
        for kind in PRUSA_KINDS {
            match list_prusa_files(&user.join(kind), MAX_PRUSA_FILES_PER_DIR) {
                Ok(files) => {
                    load_prusa_files(&mut index, &files, true, &mut budget, &mut problems);
                }
                Err(PrusaScanError::Missing) => {}
                Err(PrusaScanError::Failed) => problems.push(load_problem(
                    format!("{kind} folder"),
                    "user preset folder cannot be scanned",
                )),
            }
        }
        index.selected = read_prusa_selection(&user.join("PrusaSlicer.ini"));
    }
    index.finish();
    (index, problems)
}

fn load_problem(machine: impl Into<String>, reason: impl Into<String>) -> PrusaRejection {
    PrusaRejection {
        vendor: String::new(),
        machine: machine.into(),
        slicer: PRUSA_SLICER_LABEL,
        reason: reason.into(),
    }
}

fn load_prusa_files(
    index: &mut PrusaIndex,
    files: &[PathBuf],
    user: bool,
    budget: &mut usize,
    problems: &mut Vec<PrusaRejection>,
) {
    for file in files {
        let base = file_name_of(file);
        if *budget == 0 {
            problems.push(load_problem(base, "profile folders exceed the size budget"));
            return;
        }
        let limit = MAX_PRUSA_FILE_BYTES.min(*budget);
        let data = match read_prusa_file(file, limit) {
            Ok(data) => data,
            Err(reason) => {
                problems.push(load_problem(file_stem_like_go(&base), reason));
                continue;
            }
        };
        *budget -= data.len();
        match parse_prusa_ini(&data) {
            Ok(ini) => index.add_sections(file, user, &ini),
            Err(error) => problems.push(load_problem(
                file_stem_like_go(&base),
                format!("profile file is malformed: {error}"),
            )),
        }
    }
}

/// Reads the saved [presets] selection. A missing or unreadable PrusaSlicer.ini
/// means no selection is known.
fn read_prusa_selection(file: &Path) -> PrusaSelection {
    let Ok(data) = read_prusa_file(file, MAX_PRUSA_FILE_BYTES) else {
        return PrusaSelection::default();
    };
    let Ok(ini) = parse_prusa_ini(&data) else {
        return PrusaSelection::default();
    };
    let field = |key: &str| {
        ini.section("presets")
            .and_then(|section| section.get(key))
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };
    PrusaSelection {
        printer: field("printer"),
        print: field("print"),
        filament: field("filament"),
    }
}

impl PrusaIndex {
    /// Indexes the presets of one parsed file. Vendor names are read only from
    /// vendor bundles, and abstract `*name*` printers are never offered.
    fn add_sections(&mut self, file: &Path, user: bool, ini: &PrusaIni) {
        for header in ini.section_names() {
            let Some(section) = ini.section(header) else {
                continue;
            };
            if header == "vendor" {
                if !user {
                    let name = section.get("name").map_or("", |value| value.trim());
                    self.vendors.insert(file.to_path_buf(), name.to_owned());
                }
                continue;
            }
            let Some((kind, name)) = header.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || !PRUSA_KINDS.contains(&kind) {
                continue;
            }
            self.profiles
                .entry(format!("{kind}:{name}"))
                .or_default()
                .push(PrusaProfile {
                    file: file.to_path_buf(),
                    values: section.clone(),
                });
            if user && kind == "printer" && !name.starts_with('*') {
                self.printers.push(PrusaPrinterSource {
                    file: file.to_path_buf(),
                    name: name.to_owned(),
                });
            }
        }
    }

    /// Builds the sorted, selectable name list for each kind. Names wrapped in
    /// asterisks are abstract parents and are never chosen as a preset.
    fn finish(&mut self) {
        for key in self.profiles.keys() {
            let Some((kind, name)) = key.split_once(':') else {
                continue;
            };
            if name.starts_with('*') {
                continue;
            }
            self.names
                .entry(kind.to_owned())
                .or_default()
                .push(name.to_owned());
        }
        for names in self.names.values_mut() {
            names.sort();
        }
    }

    fn names_of(&self, kind: &str) -> &[String] {
        match self.names.get(kind) {
            Some(names) => names,
            None => &[],
        }
    }

    /// Finds one preset. It prefers the definition in `prefer`, then a single
    /// unambiguous definition. Two competing definitions fail closed.
    fn lookup(
        &self,
        kind: &str,
        name: &str,
        prefer: Option<&Path>,
    ) -> Result<&PrusaProfile, String> {
        let choices = match self.profiles.get(&format!("{kind}:{name}")) {
            Some(choices) if !choices.is_empty() => choices,
            _ => return Err(format!("{kind} {name:?} is not installed")),
        };
        if let Some(choice) =
            prefer.and_then(|file| choices.iter().find(|choice| choice.file.as_path() == file))
        {
            return Ok(choice);
        }
        match choices.as_slice() {
            [only] => Ok(only),
            _ => Err(format!("{kind} {name:?} is defined in more than one file")),
        }
    }

    /// Merges a preset with its inherited parents. Parents apply in listed order,
    /// and the preset's own keys win. Also returns every file that was used.
    fn flatten(
        &self,
        kind: &str,
        name: &str,
        prefer: Option<&Path>,
        depth: usize,
        visits: &mut usize,
    ) -> Result<(PrusaSection, Vec<PathBuf>), String> {
        *visits = visits.saturating_add(1);
        if *visits > MAX_PRUSA_INHERIT_VISITS {
            return Err(format!(
                "inheritance expands beyond {MAX_PRUSA_INHERIT_VISITS} profile visits"
            ));
        }
        if depth > MAX_PRUSA_INHERIT {
            return Err(format!(
                "inheritance is deeper than {MAX_PRUSA_INHERIT} levels"
            ));
        }
        let profile = self.lookup(kind, name, prefer)?;
        let parents = split_prusa_list(profile.values.get("inherits").map_or("", String::as_str))
            .map_err(|_| format!("{kind} {name:?} has a malformed inherits list"))?;
        let mut merged = PrusaSection::new();
        let mut origins = vec![profile.file.clone()];
        for parent in &parents {
            // A parent is looked up near the file that named it.
            let (values, parent_origins) = self.flatten(
                kind,
                parent,
                Some(profile.file.as_path()),
                depth + 1,
                visits,
            )?;
            merged.extend(values);
            origins.extend(parent_origins);
        }
        merged.extend(profile.values.clone());
        Ok((merged, origins))
    }

    fn vendor_name(&self, origins: &[PathBuf]) -> String {
        origins
            .iter()
            .filter_map(|file| self.vendors.get(file))
            .find(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| PRUSA_CUSTOM_VENDOR.to_owned())
    }

    /// Applies every fail-closed check to one user printer preset.
    fn resolve_printer(&self, source: &PrusaPrinterSource) -> Result<PrusaTarget, String> {
        let mut visits = 0;
        let (mut printer, origins) = self.flatten(
            "printer",
            &source.name,
            Some(source.file.as_path()),
            0,
            &mut visits,
        )?;
        match printer
            .get("printer_technology")
            .map_or("", |value| value.trim())
        {
            "FFF" => {}
            "" => return Err("printer_technology is missing".to_owned()),
            other => {
                return Err(format!(
                    "printer technology {} is not supported",
                    clip_prusa_text(other)
                ))
            }
        }
        if printer
            .get("bed_shape")
            .map_or("", |value| value.trim())
            .is_empty()
        {
            return Err("bed_shape is missing".to_owned());
        }
        let nozzle = prusa_nozzle(printer.get("nozzle_diameter").map_or("", String::as_str))?;
        // PrusaSlicer derives num_extruders from the nozzle list. Only single-tool
        // printers reach this point, so the value is always one.
        printer.insert("num_extruders".to_owned(), "1".to_owned());
        let vendor = self.vendor_name(&origins);
        let print = self.pick_print(&source.name, &printer, nozzle, &vendor)?;
        let filament = self.pick_filament(&source.name, &printer, &vendor)?;
        let model = match printer
            .get("printer_model")
            .map_or("", |value| value.trim())
        {
            "" => source.name.clone(),
            model => model.to_owned(),
        };
        Ok(PrusaTarget {
            id: prusa_target_id(&source.name, &source.file),
            name: source.name.clone(),
            slicer: PRUSA_SLICER_LABEL,
            source_root: "user",
            vendor,
            model,
            nozzle,
            print,
            filament,
            ready: false,
            reason: PRUSA_EXPORT_NOT_VERIFIED,
        })
    }

    /// Chooses the compatible print whose layer height is nearest half the nozzle
    /// and is no taller than 80 percent of it.
    fn pick_print(
        &self,
        printer_name: &str,
        printer: &PrusaSection,
        nozzle: &str,
        vendor: &str,
    ) -> Result<String, String> {
        let diameter = nozzle.parse::<f64>().unwrap_or(0.0);
        let mut best: Option<PrusaChoice> = None;
        for name in self.names_of("print") {
            let mut visits = 0;
            let Ok((values, origins)) = self.flatten("print", name, None, 0, &mut visits) else {
                continue;
            };
            if !matches!(prusa_compatible(&values, printer_name, printer), Ok(true)) {
                continue;
            }
            let Ok(height) = values
                .get("layer_height")
                .map_or("", |value| value.trim())
                .parse::<f64>()
            else {
                continue;
            };
            if height <= 0.0 || height > diameter * 0.8 {
                continue;
            }
            let choice = PrusaChoice {
                name: name.clone(),
                selected: self.is_selected("print", printer_name, name),
                foreign: self.vendor_name(&origins) != vendor,
                score: (height - diameter * 0.5).abs(),
            };
            if best.as_ref().map_or(true, |current| choice.beats(current)) {
                best = Some(choice);
            }
        }
        best.map(|choice| choice.name)
            .ok_or_else(|| "no compatible print preset for this nozzle".to_owned())
    }

    /// Chooses the compatible PLA filament, preferring the printer's own vendor
    /// bundle and then name order.
    fn pick_filament(
        &self,
        printer_name: &str,
        printer: &PrusaSection,
        vendor: &str,
    ) -> Result<String, String> {
        let mut best: Option<PrusaChoice> = None;
        for name in self.names_of("filament") {
            let mut visits = 0;
            let Ok((values, origins)) = self.flatten("filament", name, None, 0, &mut visits) else {
                continue;
            };
            if values.get("filament_type").map_or("", |value| value.trim()) != "PLA" {
                continue;
            }
            if !matches!(prusa_compatible(&values, printer_name, printer), Ok(true)) {
                continue;
            }
            let choice = PrusaChoice {
                name: name.clone(),
                selected: self.is_selected("filament", printer_name, name),
                foreign: self.vendor_name(&origins) != vendor,
                score: 0.0,
            };
            if best.as_ref().map_or(true, |current| choice.beats(current)) {
                best = Some(choice);
            }
        }
        best.map(|choice| choice.name)
            .ok_or_else(|| "no compatible PLA filament preset".to_owned())
    }

    /// Reports whether PrusaSlicer.ini recorded this preset as in use. The saved
    /// selection is trusted only for the printer it names.
    fn is_selected(&self, kind: &str, printer_name: &str, name: &str) -> bool {
        if self.selected.printer != printer_name {
            return false;
        }
        if kind == "print" {
            self.selected.print == name
        } else {
            self.selected.filament == name
        }
    }
}

impl PrusaChoice {
    /// The saved selection wins. Then presets from the printer's own vendor bundle.
    /// Then the lower score, and finally the name that sorts first.
    fn beats(&self, other: &PrusaChoice) -> bool {
        if self.selected != other.selected {
            return self.selected;
        }
        if self.foreign != other.foreign {
            return !self.foreign;
        }
        if self.score != other.score {
            return self.score < other.score;
        }
        self.name < other.name
    }
}

/// Applies PrusaSlicer's precedence. An explicit list decides alone. Otherwise a
/// condition decides. Otherwise every printer is compatible. An error means the
/// preset is not compatible.
fn prusa_compatible(
    preset: &PrusaSection,
    printer_name: &str,
    printer: &PrusaSection,
) -> Result<bool, String> {
    let listed = split_prusa_list(preset.get("compatible_printers").map_or("", String::as_str))?;
    if !listed.is_empty() {
        return Ok(listed.iter().any(|item| item.as_str() == printer_name));
    }
    let condition = preset
        .get("compatible_printers_condition")
        .map_or("", |value| value.trim());
    if condition.is_empty() {
        return Ok(true);
    }
    evaluate_prusa_condition(condition, printer)
}

/// Requires exactly one supported diameter. Multi-extruder printers are rejected
/// until they have a validated conversion path.
fn prusa_nozzle(raw: &str) -> Result<&'static str, String> {
    let items = split_prusa_values(raw).map_err(|_| "nozzle_diameter is malformed".to_owned())?;
    let [item] = items.as_slice() else {
        return Err(
            "nozzle_diameter must list exactly one tool; multi-extruder printers are not supported"
                .to_owned(),
        );
    };
    let value: f64 = item
        .parse()
        .map_err(|_| "nozzle_diameter is not numeric".to_owned())?;
    for supported in PRUSA_NOZZLES {
        let reference = supported.parse::<f64>().unwrap_or(f64::NAN);
        if (reference - value).abs() < 0.000001 {
            return Ok(supported);
        }
    }
    Err(format!(
        "unsupported nozzle diameter {} mm",
        clip_prusa_text(item)
    ))
}

/// Hashes the exact preset name and file, so IDs are stable for one install and
/// unique across identical names in different folders.
fn prusa_target_id(name: &str, file: &Path) -> String {
    let mut material = file.to_string_lossy().into_owned().into_bytes();
    material.push(0);
    material.extend_from_slice(name.as_bytes());
    let digest = sha256(&material);
    format!("prusa-{}-{}", prusa_slug(name, 40), hex_lower(&digest[..5]))
}

/// Lowercases to ASCII letters and digits joined by single dashes. An empty
/// result becomes "printer".
fn prusa_slug(value: &str, limit: usize) -> String {
    let mut slug = String::new();
    let mut dash = true;
    for ch in value.chars() {
        // Go lowercases each rune with its simple mapping, so U+0130 becomes 'i'.
        let lower = if ch == '\u{130}' {
            'i'
        } else {
            ch.to_lowercase().next().unwrap_or(ch)
        };
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            slug.push(lower);
            dash = false;
        } else if !dash {
            slug.push('-');
            dash = true;
        }
    }
    let mut slug = slug.trim_matches('-').to_owned();
    if slug.len() > limit {
        slug.truncate(limit);
        slug = slug.trim_matches('-').to_owned();
    }
    if slug.is_empty() {
        return "printer".to_owned();
    }
    slug
}

/// Returns the first `LIMIT` bytes of a string, backing off to a character
/// boundary so the result stays valid UTF-8.
fn clip_prusa_text(text: &str) -> &str {
    const LIMIT: usize = 32;
    if text.len() <= LIMIT {
        return text;
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Lists regular `.ini` files in one folder, sorted by name as Go's `os.ReadDir`
/// does. Symlinks, junctions and other special entries are skipped, and the
/// count is bounded.
fn list_prusa_files(dir: &Path, limit: usize) -> Result<Vec<PathBuf>, PrusaScanError> {
    let entries = fs::read_dir(dir).map_err(|error| match error.kind() {
        ErrorKind::NotFound => PrusaScanError::Missing,
        _ => PrusaScanError::Failed,
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| PrusaScanError::Failed)?;
        // `file_type` does not follow symlinks, so links fail the regular-file check.
        let regular = entry.file_type().is_ok_and(|kind| kind.is_file());
        let name = entry.file_name().to_string_lossy().into_owned();
        if regular && has_ini_extension(&name) {
            files.push(entry.path());
        }
    }
    files.sort();
    if files.len() > limit {
        return Err(PrusaScanError::Failed);
    }
    Ok(files)
}

/// Matches Go's case-insensitive `filepath.Ext(name) == ".ini"`.
fn has_ini_extension(name: &str) -> bool {
    name.len() >= 4
        && name
            .get(name.len() - 4..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(".ini"))
}

/// Reads one regular file no larger than `limit`. Error text never contains the path.
fn read_prusa_file(file: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let info = fs::symlink_metadata(file).map_err(|_| PRUSA_UNREADABLE.to_owned())?;
    if !info.is_file() || info.len() > limit as u64 {
        return Err(PRUSA_UNREADABLE.to_owned());
    }
    let handle = File::open(file).map_err(|_| PRUSA_UNREADABLE.to_owned())?;
    let mut data = Vec::new();
    handle
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| PRUSA_UNREADABLE.to_owned())?;
    if data.len() > limit {
        return Err(PRUSA_UNREADABLE.to_owned());
    }
    Ok(data)
}

fn file_name_of(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Strips the extension the way Go's `strings.TrimSuffix(base, filepath.Ext(base))` does.
fn file_stem_like_go(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) => &name[..dot],
        None => name,
    }
}

/// SHA-256 (FIPS 180-4). Implemented here so target IDs match the Go build without
/// adding a dependency.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
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
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
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
        for (current, add) in state.iter_mut().zip(v) {
            *current = current.wrapping_add(add);
        }
    }

    let mut digest = [0u8; 32];
    for (chunk, word) in digest.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const QIDI_VENDOR_FIXTURE: &str = r##"# fixture
[vendor]
name = QIDITechnology

[printer:*common*]
bed_shape = 0x0,350x0,350x350,0x350
printer_technology = FFF
printer_model =
nozzle_diameter = 0.4
machine_start_gcode = G28

[printer:*X-Plus 4*]
inherits = *common*
printer_model = X-Plus 4

[printer:X-Plus 4 0.4 nozzle]
inherits = *X-Plus 4*
nozzle_diameter = 0.4

[print:*common*]
layer_height = 0.2
first_layer_height = 0.2

[print:0.20mm Standard @X-Plus 4]
inherits = *common*
compatible_printers_condition = printer_model=="X-Plus 4"

[print:0.12mm Fine @X-Plus 4]
inherits = *common*
layer_height = 0.12
compatible_printers_condition = printer_model=="X-Plus 4" and nozzle_diameter[0]==0.4

[print:0.20mm Other printer]
inherits = *common*
compatible_printers = "Some Other Printer"

[print:0.20mm regex only]
inherits = *common*
compatible_printers_condition = printer_notes=~/.*PRINTER_VENDOR_QIDI.*/

[filament:*common*]
filament_type = PLA

[filament:Generic PLA @X-Plus 4]
inherits = *common*
compatible_printers_condition = printer_model=="X-Plus 4"

[filament:Generic ABS @X-Plus 4]
filament_type = ABS
compatible_printers_condition = printer_model=="X-Plus 4"
"##;

    const USER_QIDI_PRINTER: &str =
        "[printer:QIDI Plus5 0.4 nozzle]\ninherits = X-Plus 4 0.4 nozzle\n";

    /// A temporary folder that is removed when the test ends. Each instance gets a
    /// unique name, so tests can run in parallel.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "slicebridge-prusa-{label}-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Writes or overwrites a file and creates its parent folders.
        fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("parent folder")).expect("create parent");
            fs::write(&path, content).expect("write fixture");
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn qidi_fixture(label: &str) -> (TestDir, PrusaLocations) {
        let root = TestDir::new(label);
        root.write("vendor/QIDITechnology.ini", QIDI_VENDOR_FIXTURE);
        root.write("user/printer/QIDI Plus5 0.4 nozzle.ini", USER_QIDI_PRINTER);
        let loc = PrusaLocations {
            user: Some(root.path().join("user")),
            vendor: Some(root.path().join("vendor")),
        };
        (root, loc)
    }

    fn section(pairs: &[(&str, &str)]) -> PrusaSection {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn discovers_complete_printer_with_source_presets() {
        let (root, loc) = qidi_fixture("discover");
        let discovery = discover_prusa_at(&loc);
        assert!(
            discovery.rejected.is_empty(),
            "unexpected rejections: {:?}",
            discovery.rejected
        );
        assert_eq!(discovery.targets.len(), 1);
        let target = &discovery.targets[0];
        assert_eq!(target.name, "QIDI Plus5 0.4 nozzle");
        assert_eq!(target.vendor, "QIDITechnology");
        assert_eq!(target.model, "X-Plus 4");
        assert_eq!(target.nozzle, "0.4");
        assert_eq!(target.slicer, "PrusaSlicer");
        // Both prints are compatible. The one whose layer height is nearest half
        // the nozzle wins, and the PLA filament is picked by name.
        assert_eq!(target.print, "0.20mm Standard @X-Plus 4");
        assert_eq!(target.filament, "Generic PLA @X-Plus 4");
        assert!(is_prusa_target_id(&target.id));
        assert!(!target.ready);
        assert_eq!(target.reason, PRUSA_EXPORT_NOT_VERIFIED);
        let root_text = root.path().to_string_lossy().into_owned();
        assert!(!format!("{discovery:?}").contains(root_text.as_str()));
    }

    #[test]
    fn target_ids_are_stable_and_tied_to_the_exact_preset() {
        let (root, loc) = qidi_fixture("ids");
        let first = discover_prusa_at(&loc).targets[0].id.clone();
        assert_eq!(
            discover_prusa_at(&loc).targets[0].id,
            first,
            "ID changed between runs"
        );
        root.write(
            "elsewhere/printer/QIDI Plus5 0.4 nozzle.ini",
            USER_QIDI_PRINTER,
        );
        let elsewhere = PrusaLocations {
            user: Some(root.path().join("elsewhere")),
            vendor: loc.vendor.clone(),
        };
        let moved = discover_prusa_at(&elsewhere).targets[0].id.clone();
        assert_ne!(
            moved, first,
            "same preset name in another folder reused the ID"
        );
        assert!(
            first.starts_with("prusa-qidi-plus5-0-4-nozzle-"),
            "unexpected ID shape: {first}"
        );
        assert!(is_prusa_target_id(&first));
    }

    #[test]
    fn rejects_malformed_and_unsupported_printers_without_hiding_good_ones() {
        let (root, loc) = qidi_fixture("reject");
        let printers = [
            ("sla.ini", "[printer:SLA]\ninherits = X-Plus 4 0.4 nozzle\nprinter_technology = SLA\n"),
            ("two tools.ini", "[printer:Two tools]\ninherits = X-Plus 4 0.4 nozzle\nnozzle_diameter = 0.4,0.4\n"),
            ("half.ini", "[printer:Half]\ninherits = X-Plus 4 0.4 nozzle\nnozzle_diameter = 0.5\n"),
            ("no bed.ini", "[printer:No bed]\nprinter_technology = FFF\nbed_shape =\nnozzle_diameter = 0.4\n"),
            ("broken.ini", "[printer:Broken]\nnot a setting\n"),
            ("orphan.ini", "[printer:Orphan]\ninherits = Missing Parent\n"),
            ("self loop.ini", "[printer:Loop]\ninherits = Loop\n"),
            ("lonely.ini", "[printer:Lonely]\nprinter_technology = FFF\nbed_shape = 0x0,1x0,1x1\nnozzle_diameter = 0.4\nprinter_model = Lonely\n"),
            ("duplicate.ini", "[printer:Dup]\nnozzle_diameter = 0.4\n[printer:Dup]\n"),
        ];
        for (name, content) in printers {
            root.write(&format!("user/printer/{name}"), content);
        }
        let discovery = discover_prusa_at(&loc);
        assert_eq!(discovery.targets.len(), 1, "a bad printer hid the good one");
        assert_eq!(discovery.targets[0].name, "QIDI Plus5 0.4 nozzle");

        let reasons: BTreeMap<&str, &str> = discovery
            .rejected
            .iter()
            .map(|rejected| (rejected.machine.as_str(), rejected.reason.as_str()))
            .collect();
        let expect = [
            ("SLA", "not supported"),
            ("Two tools", "multi-extruder"),
            ("Half", "unsupported nozzle"),
            ("No bed", "bed_shape"),
            ("Orphan", "not installed"),
            ("Lonely", "no compatible print"),
            ("Loop", "deeper than"),
            ("broken", "malformed"),
            ("duplicate", "malformed"),
        ];
        for (machine, fragment) in expect {
            let reason = reasons.get(machine).copied().unwrap_or_default();
            assert!(
                reason.contains(fragment),
                "{machine}: reason {reason:?} lacks {fragment:?}"
            );
        }
        let root_text = root.path().to_string_lossy().into_owned();
        for rejected in &discovery.rejected {
            assert!(
                !rejected.reason.contains(root_text.as_str()),
                "rejection leaks an install path: {:?}",
                rejected.reason
            );
        }
    }

    #[test]
    fn skips_symlinks_and_reports_oversized_vendor_files() {
        let (root, loc) = qidi_fixture("links");
        #[cfg(unix)]
        {
            // A valid printer outside the scanned folder. Following the link would add a target.
            let outside = root.write("outside.ini", USER_QIDI_PRINTER);
            std::os::unix::fs::symlink(outside, root.path().join("user/printer/linked.ini"))
                .expect("create symlink");
        }
        root.write("vendor/huge.ini", &"#".repeat(MAX_PRUSA_FILE_BYTES + 1));
        let discovery = discover_prusa_at(&loc);
        assert_eq!(
            discovery.targets.len(),
            1,
            "symlink or oversized file changed targets"
        );
        assert!(
            discovery
                .rejected
                .iter()
                .any(|rejected| rejected.machine == "huge" && rejected.reason == PRUSA_UNREADABLE),
            "oversized vendor file not reported: {:?}",
            discovery.rejected
        );
    }

    #[test]
    fn missing_folders_yield_empty_discovery() {
        let root = TestDir::new("missing");
        let loc = PrusaLocations {
            user: Some(root.path().join("none")),
            vendor: Some(root.path().join("none2")),
        };
        assert_eq!(discover_prusa_at(&loc), PrusaDiscovery::default());
    }

    #[test]
    fn compatibility_follows_prusaslicer_precedence() {
        let printer = section(&[("printer_model", "X-Plus 4")]);
        let cases = [
            (
                "no list and no condition applies everywhere",
                section(&[]),
                true,
            ),
            (
                "explicit list names this printer",
                section(&[("compatible_printers", r#""Mine";"Other""#)]),
                true,
            ),
            (
                "explicit list omits this printer",
                section(&[("compatible_printers", r#""Other""#)]),
                false,
            ),
            (
                "explicit list beats a matching condition",
                section(&[
                    ("compatible_printers", r#""Other""#),
                    (
                        "compatible_printers_condition",
                        r#"printer_model=="X-Plus 4""#,
                    ),
                ]),
                false,
            ),
            (
                "condition decides when the list is empty",
                section(&[(
                    "compatible_printers_condition",
                    r#"printer_model=="X-Plus 4""#,
                )]),
                true,
            ),
        ];
        for (name, preset, want) in cases {
            assert_eq!(
                prusa_compatible(&preset, "Mine", &printer),
                Ok(want),
                "{name}"
            );
        }
        let regex = section(&[("compatible_printers_condition", "printer_model=~/^X-Plus/")]);
        assert_eq!(prusa_compatible(&regex, "Mine", &printer), Ok(true));
        let bad_regex = section(&[("compatible_printers_condition", "printer_notes=~/[/")]);
        assert!(
            prusa_compatible(&bad_regex, "Mine", &printer).is_err(),
            "invalid regex did not fail closed"
        );
    }

    #[test]
    fn resolve_only_returns_completely_resolved_installed_presets() {
        let (_root, loc) = qidi_fixture("resolve");
        let id = discover_prusa_at(&loc).targets[0].id.clone();
        let resolved = resolve_prusa_target_at(&loc, &id).map(|target| target.id);
        assert_eq!(resolved, Ok(id));
        for bad in [
            "prusa-project",
            "prusa-../x",
            "PRUSA-QIDI-0123456789",
            "prusa-qidi-0123456789",
            "",
        ] {
            assert!(
                resolve_prusa_target_at(&loc, bad).is_err(),
                "{bad:?} resolved"
            );
        }
        assert!(
            resolve_prusa_target_at(&loc, "prusa-qidi-plus5-0-4-nozzle-0000000000").is_err(),
            "well-formed but uninstalled ID resolved"
        );
    }

    #[test]
    fn inheritance_expansion_has_a_bounded_visit_budget() {
        let mut index = PrusaIndex::default();
        for depth in 0..=MAX_PRUSA_INHERIT {
            let mut values = PrusaSection::new();
            if depth < MAX_PRUSA_INHERIT {
                values.insert(
                    "inherits".into(),
                    vec![format!("node{}", depth + 1); 4].join(";"),
                );
            }
            index.profiles.insert(
                format!("printer:node{depth}"),
                vec![PrusaProfile {
                    file: PathBuf::from(format!("node{depth}.ini")),
                    values,
                }],
            );
        }
        let mut visits = 0;
        let error = index
            .flatten("printer", "node0", None, 0, &mut visits)
            .unwrap_err();
        assert!(error.contains("4096"));
        assert_eq!(visits, MAX_PRUSA_INHERIT_VISITS + 1);
    }

    #[test]
    fn saved_selection_ranks_only_for_its_own_printer() {
        let (root, loc) = qidi_fixture("selection");
        let print_of = |loc: &PrusaLocations| discover_prusa_at(loc).targets[0].print.clone();
        root.write(
            "user/PrusaSlicer.ini",
            "[presets]\nprinter = QIDI Plus5 0.4 nozzle\nprint = 0.12mm Fine @X-Plus 4\n",
        );
        assert_eq!(print_of(&loc), "0.12mm Fine @X-Plus 4");
        root.write(
            "user/PrusaSlicer.ini",
            "[presets]\nprinter = Someone Else\nprint = 0.12mm Fine @X-Plus 4\n",
        );
        assert_eq!(print_of(&loc), "0.20mm Standard @X-Plus 4");
        root.write("user/PrusaSlicer.ini", "[presets\nbroken\n");
        assert_eq!(
            discover_prusa_at(&loc).targets.len(),
            1,
            "corrupt selection hid the printer"
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            hex_lower(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_lower(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 56 bytes: the padding spills into a second block.
        assert_eq!(
            hex_lower(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn slugs_follow_the_go_shape() {
        assert_eq!(
            prusa_slug("QIDI Plus5 0.4 nozzle", 40),
            "qidi-plus5-0-4-nozzle"
        );
        assert_eq!(
            prusa_slug("  \u{c4}\u{d6} -- Prusa MK4S (Pro)!  ", 40),
            "prusa-mk4s-pro"
        );
        assert_eq!(prusa_slug("!!!", 40), "printer");
        assert_eq!(prusa_slug("a-b-c", 3), "a-b");
        assert_eq!(prusa_slug("\u{130}stanbul", 40), "istanbul");
    }

    #[test]
    fn target_id_shape_matches_go_pattern() {
        assert!(is_prusa_target_id("prusa-qidi-0123456789"));
        assert!(is_prusa_target_id("prusa-qidi-plus5-0-4-nozzle-0123456789"));
        for bad in [
            "prusa-project",
            "prusa-../x",
            "PRUSA-QIDI-0123456789",
            "prusa-qidi-0123456789A",
            "prusa--0123456789",
            "prusa-a--0123456789",
            "prusa-qidi-012345678",
            "prusa-",
            "",
        ] {
            assert!(!is_prusa_target_id(bad), "{bad:?} accepted");
        }
    }

    #[test]
    fn lists_only_regular_ini_files_in_name_order() {
        let root = TestDir::new("listing");
        root.write("folder/b.ini", "");
        root.write("folder/A.INI", "");
        root.write("folder/.ini", "");
        root.write("folder/notes.txt", "");
        root.write("folder/c.ini.bak", "");
        root.write("folder/nested.ini/x.txt", "");
        let folder = root.path().join("folder");
        let names: Vec<String> = list_prusa_files(&folder, 10)
            .expect("list folder")
            .iter()
            .map(|path| file_name_of(path))
            .collect();
        assert_eq!(names, [".ini", "A.INI", "b.ini"]);
        assert_eq!(list_prusa_files(&folder, 2), Err(PrusaScanError::Failed));
        assert_eq!(
            list_prusa_files(&root.path().join("absent"), 10),
            Err(PrusaScanError::Missing)
        );
    }
}
