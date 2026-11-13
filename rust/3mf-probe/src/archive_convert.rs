//! Experimental 3MF retargeting assembly from independently verified
//! conversion components. This is not connected to the installed application.
//! An explicit caller-provided target profile and mapping are mandatory;
//! outputs are created as new files and never submitted to a printer.
use crate::{
    compose_project_settings, inspect_package, rewrite_model_paint_xml, rewrite_model_settings,
    rewrite_slice_info_xml, strip_slice_cache_preserving, SliceInfoSlot, MAX_COMPRESSED_BYTES,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_XML: u64 = 8 << 20;
const MAX_PRESERVED: usize = 64;
static NEXT_OUTPUT: AtomicU64 = AtomicU64::new(0);
type FilamentMapping = (BTreeMap<u32, u32>, BTreeMap<i32, i32>);

struct Stage(PathBuf);

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// The original path can be rewritten while a conversion is in progress.
// Validation, source-material auditing, XML transformation and the final
// geometry comparison MUST read one immutable scratch snapshot, never
// independently reopen the original source path across stages.
//
// The snapshot uses a new file in the already approved scratch output
// directory; it never modifies the source or installed slicer profiles.
fn fresh_stage_path(parent: &Path, label: &str) -> Result<PathBuf, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    Ok(parent.join(format!(
        ".slicebridge-{label}-{}-{now}-{}.partial",
        process::id(),
        NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
    )))
}

fn snapshot_source(source: &Path, parent: &Path) -> Result<Stage, String> {
    let input = File::open(source).map_err(|e| format!("source open: {e}"))?;
    let meta = input
        .metadata()
        .map_err(|e| format!("source metadata: {e}"))?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_COMPRESSED_BYTES {
        return Err("3MF must be a nonempty regular file up to 250 MB".into());
    }
    let snapshot = Stage(fresh_stage_path(parent, "source")?);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&snapshot.0)
        .map_err(|e| format!("cannot create private source snapshot: {e}"))?;
    let bytes = std::io::copy(&mut input.take(MAX_COMPRESSED_BYTES + 1), &mut file)
        .map_err(|e| format!("source snapshot: {e}"))?;
    if bytes != meta.len() || bytes > MAX_COMPRESSED_BYTES {
        return Err("source changed size while being captured; refusing conversion".into());
    }
    file.sync_all()
        .map_err(|e| format!("source snapshot sync: {e}"))?;
    Ok(snapshot)
}

fn read_part(mut entry: impl Read, label: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    entry
        .by_ref()
        .take(MAX_XML + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("reading {label}: {e}"))?;
    if data.len() as u64 > MAX_XML {
        return Err(format!("{label} exceeds bounded XML/JSON limit"));
    }
    Ok(data)
}

fn mapping(plan: &Value, slot_count: usize) -> Result<FilamentMapping, String> {
    let items = plan
        .get("mapping")
        .and_then(Value::as_object)
        .ok_or_else(|| "conversion mapping must be an object".to_owned())?;
    if items.is_empty() || items.len() > 64 {
        return Err("mapping must contain 1..64 source filament IDs".into());
    }
    let mut paint = BTreeMap::new();
    let mut xml = BTreeMap::new();
    for (original, slot) in items {
        let canonical = original.as_str();
        let original: u32 = canonical
            .parse()
            .map_err(|_| "mapping source ID must be a positive integer".to_owned())?;
        if canonical != original.to_string() {
            return Err(
                "source filament ID must be canonical decimal, without sign or leading zero".into(),
            );
        }
        let dst = slot
            .as_u64()
            .ok_or_else(|| "mapping destination must be a positive integer".to_owned())?;
        if original == 0 || original > i32::MAX as u32 || !(1..=slot_count as u64).contains(&dst) {
            return Err("source or destination mapping ID outside supported range".into());
        }
        paint.insert(original, dst as u32);
        xml.insert(original as i32, dst as i32);
    }
    Ok((paint, xml))
}

fn source_attribute(start: &BytesStart<'_>, name: &str) -> Result<Option<String>, String> {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| format!("source XML attribute: {e}"))?;
        if attribute.key.local_name().as_ref() == name {
            let decoded = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|e| format!("source XML attribute {name}: {e}"))?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

/// A missing paint ID must not silently turn into another printer material.
/// Match the production Go source-material-reference guard before producing
/// any output: all declared filament IDs must be mapped, no fabricated IDs,
/// every painted leaf and positive object/part default must be declared.
fn audit_source_material_references(
    source: &mut ZipArchive<File>,
    mapping: &BTreeMap<u32, u32>,
) -> Result<(), String> {
    let original = source
        .by_name("Metadata/slice_info.config")
        .map_err(|_| "input 3MF has no declared slice_info filament palette".to_owned())?;
    let xml = read_part(original, "source slice-info palette")?;
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut declared = BTreeSet::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("source slice-info XML: {e}"))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start)
                if start.local_name().as_ref() == "filament" =>
            {
                if let Some(id) = source_attribute(start, "id")? {
                    if let Ok(id) = id.parse::<u32>() {
                        if id > 0 {
                            declared.insert(id);
                        }
                    }
                }
            }
            Event::Eof => break,
            Event::DocType(_) => return Err("source slice-info DOCTYPE disallowed".into()),
            _ => {}
        }
    }
    if declared.is_empty() {
        return Err("input 3MF slice-info contains no positive filament IDs".into());
    }
    if declared != mapping.keys().copied().collect::<BTreeSet<_>>() {
        return Err(
            "explicit conversion mapping must cover exactly the declared source filament IDs"
                .into(),
        );
    }

    let mut missing_painted = BTreeSet::new();
    let mut missing_defaults = BTreeSet::new();
    for index in 0..source.len() {
        let entry = source.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        let model = name.to_ascii_lowercase().ends_with(".model");
        let defaults = name == "Metadata/model_settings.config";
        if !model && !defaults {
            continue;
        }
        let data = read_part(entry, &name)?;
        let mut xml = Reader::from_reader(data.as_slice());
        loop {
            let event = xml
                .read_event()
                .map_err(|e| format!("{name}: invalid XML: {e}"))?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start)
                    if model && start.local_name().as_ref() == "triangle" =>
                {
                    for attribute in start.attributes() {
                        let attribute = attribute.map_err(|e| format!("{name}: {e}"))?;
                        if !matches!(
                            attribute.key.local_name().as_ref(),
                            "paint_color" | "mmu_segmentation"
                        ) {
                            continue;
                        }
                        let encoded = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|e| format!("{name}: invalid paint attribute: {e}"))?;
                        if encoded.is_empty() {
                            continue;
                        }
                        for painted in crate::paint::inspect_paint_ids(&encoded)? {
                            if !declared.contains(&painted) {
                                missing_painted.insert(painted);
                            }
                        }
                    }
                }
                Event::Start(ref start) | Event::Empty(ref start)
                    if defaults && start.local_name().as_ref() == "metadata" =>
                {
                    if source_attribute(start, "key")?.as_deref() == Some("extruder") {
                        if let Some(value) = source_attribute(start, "value")? {
                            if let Ok(id) = value.trim().parse::<u32>() {
                                if id > 0 && !declared.contains(&id) {
                                    missing_defaults.insert(id);
                                }
                            }
                        }
                    }
                }
                Event::DocType(_) => return Err(format!("{name}: DOCTYPE disallowed")),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    if !missing_painted.is_empty() || !missing_defaults.is_empty() {
        return Err(format!(
            "source references undeclared filament IDs: painted={missing_painted:?}, defaults={missing_defaults:?}"
        ));
    }
    Ok(())
}

/// Retarget a source 3MF from an explicit, already-vetted conversion plan.
///
/// Safety: validates source, source-settings snapshot, mapping and each input
/// ZIP member before publishing a brand-new output; no output is installed,
/// opened by a slicer, or sent to any printer. The source is immutable.
///
/// Request fields: profile (Go printer.Profile JSON), slots (Go []Slot),
/// slot_count, supports, source_settings, mapping, optional preserved.
pub fn convert_archive_experimental(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    plan: &Value,
) -> Result<usize, String> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    if destination.exists() {
        return Err("destination already exists; refusing overwrite".into());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "destination must include a parent directory".to_owned())?;
    if !parent.is_dir() {
        return Err("destination directory does not exist".into());
    }
    // Freeze the input once; the source path may be replaced during an
    // ordinary long conversion. All subsequent operations use this same
    // private scratch snapshot, including central-directory verification.
    let captured = snapshot_source(source, parent)?;
    // Apply the same source-archive checks as the standalone, read-only
    // 3MF inspector (unsafe names, duplicates, limits, required geometry).
    let source_facts = inspect_package(&captured.0)?;

    let slot_count = plan
        .get("slot_count")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "slot_count must be an integer".to_owned())?;
    if !(1..=64).contains(&slot_count) {
        return Err("slot_count outside 1..64".into());
    }
    let profile = plan
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| "profile must be a verified snapshot object".to_owned())?;
    let model_id = profile
        .get("printer_model_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() < 256)
        .ok_or_else(|| "target printer_model_id missing or oversized".to_owned())?;
    let declared_slots = profile
        .get("filament_slots")
        .and_then(Value::as_u64)
        .ok_or_else(|| "profile.filament_slots must be an integer".to_owned())?;
    if declared_slots != slot_count as u64 {
        return Err("target profile filament_slots does not match conversion slot_count".into());
    }
    if profile
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("target printer profile name missing".into());
    }
    let source_settings = plan
        .get("source_settings")
        .and_then(Value::as_object)
        .ok_or_else(|| "source_settings must be a JSON object".to_owned())?;
    let slots = plan
        .get("slots")
        .and_then(Value::as_array)
        .ok_or_else(|| "slots must be an array".to_owned())?;
    if slots.is_empty() || slots.len() > slot_count {
        return Err("slots outside declared printer capacity".into());
    }
    // A mapped source may only choose an explicitly declared material slot,
    // never an automatically padded empty hardware bay.
    let (paint_map, xml_map) = mapping(plan, slots.len())?;
    let mut padded = Vec::with_capacity(slot_count);
    for index in 0..slot_count {
        if let Some(slot) = slots.get(index) {
            let color = slot
                .get("color")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("slot {index}: invalid color"))?;
            let material = slot
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("slot {index}: invalid material"))?;
            if color.is_empty() || material.is_empty() {
                return Err(format!("slot {index} is missing a color or material"));
            }
            padded.push(SliceInfoSlot {
                color: color.to_owned(),
                material: material.to_owned(),
            });
        } else {
            padded.push(SliceInfoSlot {
                color: "#FFFFFF".to_owned(),
                material: "PLA".to_owned(),
            });
        }
    }
    let preserved: Vec<String> = match plan.get("preserved").and_then(Value::as_array) {
        Some(values) if values.len() <= MAX_PRESERVED => values
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128 && !s.contains(';'))
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "preserved setting key invalid".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
        _ => return Err("too many preserved settings".into()),
    };
    let settings = compose_project_settings(plan)?;
    let native_profiles = settings
        .get("filament_settings_id")
        .and_then(Value::as_array)
        .ok_or_else(|| "target settings lack installed filament profile IDs".to_owned())?;
    if native_profiles
        .iter()
        .any(|id| id.as_str().is_none_or(str::is_empty))
    {
        return Err("target settings resolve to an empty filament profile ID".into());
    }
    let project_settings_bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|e| format!("project settings encoding: {e}"))?;

    // The plan must describe the actual source project, not stale or
    // unrelated metadata from another printer/project.
    let mut src = ZipArchive::new(File::open(&captured.0).map_err(|e| e.to_string())?)
        .map_err(|e| format!("input snapshot archive: {e}"))?;
    let mut actual_source = serde_json::Map::new();
    if let Ok(entry) = src.by_name("Metadata/project_settings.config") {
        let bytes = read_part(entry, "source project settings")?;
        actual_source = serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| format!("source project settings JSON: {e}"))?
            .as_object()
            .cloned()
            .ok_or_else(|| "source project settings are not a JSON object".to_owned())?;
    }
    if actual_source != *source_settings {
        return Err("source_settings snapshot differs from actual input 3MF".into());
    }
    audit_source_material_references(&mut src, &paint_map)?;

    let stage = Stage(fresh_stage_path(parent, "convert")?);
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&stage.0)
        .map_err(|e| format!("cannot open temporary output: {e}"))?;
    let mut dst = ZipWriter::new(file);
    let mut had_settings = false;
    let mut had_slice_info = false;

    for index in 0..src.len() {
        let mut entry = src.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        let edited = match name.as_str() {
            "Metadata/project_settings.config" => {
                had_settings = true;
                Some(project_settings_bytes.clone())
            }
            "Metadata/model_settings.config" => {
                let bytes = read_part(entry.by_ref(), &name)?;
                Some(rewrite_model_settings(&bytes, &xml_map, slot_count)?)
            }
            "Metadata/slice_info.config" => {
                had_slice_info = true;
                let bytes = read_part(entry.by_ref(), &name)?;
                Some(rewrite_slice_info_xml(&bytes, &padded, &xml_map, model_id)?)
            }
            _ if name.ends_with(".model") => {
                let bytes = read_part(entry.by_ref(), &name)?;
                Some(rewrite_model_paint_xml(&bytes, &paint_map)?)
            }
            _ => None,
        };

        if let Some(bytes) = edited {
            let options = entry.options();
            dst.start_file(&name, options)
                .map_err(|e| format!("3MF ZIP output {name}: {e}"))?;
            dst.write_all(&bytes)
                .map_err(|e| format!("3MF ZIP write {name}: {e}"))?;
        } else {
            drop(entry);
            let raw = src.by_index(index).map_err(|e| e.to_string())?;
            dst.raw_copy_file(raw)
                .map_err(|e| format!("3MF ZIP raw copy {name}: {e}"))?;
        }
    }

    if !had_settings {
        dst.start_file(
            "Metadata/project_settings.config",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .map_err(|e| format!("project settings ZIP member: {e}"))?;
        dst.write_all(&project_settings_bytes)
            .map_err(|e| format!("project settings ZIP write: {e}"))?;
    }
    if !had_slice_info {
        return Err("source 3MF lacks slice_info.config; cannot verify target usage".into());
    }
    let output = dst.finish().map_err(|e| format!("3MF ZIP finalize: {e}"))?;
    output
        .sync_all()
        .map_err(|e| format!("3MF ZIP sync: {e}"))?;
    drop(output);

    // Inspect the exact staged ZIP before atomic publication. This verifies
    // that the package still contains editable geometry, no unsafe members,
    // and the same mesh counts as the source even if the source path changes.
    let staged = inspect_package(&stage.0)?;
    if source_facts.vertices != staged.vertices || source_facts.triangles != staged.triangles {
        return Err("staged 3MF does not preserve source mesh geometry counts".into());
    }

    // Strip old G-code and references only after conversion, exactly as the
    // real Go engine does. This also atomically installs the final file and
    // refuses to overwrite an existing destination.
    strip_slice_cache_preserving(&stage.0, destination, &preserved)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slicebridge-rust-snapshot-{}-{}",
            process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).expect("create private scratch dir");
        dir
    }

    #[test]
    fn converter_snapshot_is_independent_of_source_path_replacement() {
        let dir = scratch_dir();
        let source = dir.join("original.3mf");
        fs::write(&source, b"original immutable source bytes").unwrap();
        let snapshot = snapshot_source(&source, &dir).expect("capture bounded source");
        let snapshot_path = snapshot.0.clone();
        assert!(snapshot_path.exists());
        assert_ne!(snapshot_path, source);

        // The user may independently replace a project after conversion starts.
        // Auditing and transforming the frozen snapshot must be deterministic.
        fs::write(&source, b"replaced by another editor").unwrap();
        assert_eq!(
            fs::read(&snapshot_path).unwrap(),
            b"original immutable source bytes"
        );
        assert_eq!(fs::read(&source).unwrap(), b"replaced by another editor");

        drop(snapshot);
        assert!(!snapshot_path.exists(), "source snapshot was not cleaned");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn converter_rejects_empty_sources_without_leaking_scratch_files() {
        let dir = scratch_dir();
        let source = dir.join("empty.3mf");
        fs::write(&source, b"").unwrap();
        assert!(snapshot_source(&source, &dir).is_err());
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "snapshot left stray partial files");
        fs::remove_dir_all(&dir).unwrap();
    }
}
