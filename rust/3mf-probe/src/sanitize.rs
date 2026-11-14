//! Experimental 3MF cache stripping, independent from production Go conversion.
//! Source is never written; the output is installed atomically without overwrite.
use crate::qidi_origin::set_qidi_application_metadata;
use crate::{is_cache, safe_entry_name, verify_central_directory};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer, XmlVersion};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::ZipArchive;
use zip::ZipWriter;

const MAX_COMPRESSED_BYTES: u64 = 250 << 20;
const MAX_UNCOMPRESSED_BYTES: u64 = 1024 << 20;
const MAX_ENTRIES: usize = 10_000;
const MAX_XML_BYTES: u64 = 8 << 20;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TemporaryOutput(PathBuf);
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Remove cached slicer artifacts and OPC references, without changing
/// printer settings. Equivalent to Go stripSliceCache(source, dest, nil).
pub fn strip_slice_cache(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<usize, String> {
    strip_slice_cache_preserving(source, destination, &[])
}

/// Remove slice caches and preserve named process overrides in Bambu metadata.
/// Equivalent to Go stripSliceCache(source, dest, preserved).
///
/// Output is written to a unique sibling temporary file, then linked into
/// place without overwriting an existing destination.
pub fn strip_slice_cache_preserving(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    preserved: &[String],
) -> Result<usize, String> {
    strip_slice_cache_preserving_for_qidi(source, destination, preserved, false)
}

// Shared final ZIP stage for experimental native QIDI conversion. Existing
// non-QIDI callers retain the original byte-for-byte cache cleanup behavior.
pub(crate) fn strip_slice_cache_preserving_for_qidi(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    preserved: &[String],
    qidi: bool,
) -> Result<usize, String> {
    let source = source.as_ref();
    let destination = destination.as_ref();

    let metadata = fs::metadata(source).map_err(|e| format!("source: {e}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_COMPRESSED_BYTES {
        return Err("3MF must be a nonempty regular file up to 250 MB".to_owned());
    }
    if destination.exists() {
        return Err("destination already exists; refusing to overwrite".to_owned());
    }
    let dest_parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "destination must include a parent directory".to_owned())?;
    if !dest_parent.is_dir() {
        return Err("destination directory does not exist".to_owned());
    }

    let mut src = ZipArchive::new(File::open(source).map_err(|e| format!("cannot open 3MF: {e}"))?)
        .map_err(|e| format!("invalid 3MF ZIP: {e}"))?;
    if src.len() > MAX_ENTRIES {
        return Err("archive contains too many entries".to_owned());
    }
    verify_central_directory(source, src.central_directory_start(), src.len())?;

    let mut seen = HashSet::new();
    let mut total = 0u64;
    for index in 0..src.len() {
        let entry = src.by_index_raw(index).map_err(|e| e.to_string())?;
        if !safe_entry_name(entry.name()) || !seen.insert(entry.name().to_owned()) {
            return Err("unsafe or duplicate ZIP entry".to_owned());
        }
        total = total
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_UNCOMPRESSED_BYTES)
            .ok_or_else(|| "archive exceeds decompressed size limit".to_owned())?;
    }

    let parent = dest_parent.canonicalize().map_err(|e| e.to_string())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let ordinal = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(
        ".slicebridge-rust-{}-{now}-{ordinal}.partial",
        std::process::id()
    ));
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("cannot create temporary output: {e}"))?;
    let cleanup = TemporaryOutput(tmp);
    let mut dst = ZipWriter::new(file);
    let mut removed = 0usize;
    let mut marked_qidi_model = false;

    for index in 0..src.len() {
        let mut entry = src.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        if is_cache(&name) {
            removed += 1;
            continue;
        }
        if qidi && name == "3D/3dmodel.model" {
            if entry.size() > 200 << 20 {
                return Err("QIDI model XML exceeds 200 MiB".into());
            }
            let mut data = Vec::new();
            entry
                .by_ref()
                .take((200 << 20) + 1)
                .read_to_end(&mut data)
                .map_err(|e| format!("QIDI model XML read: {e}"))?;
            if data.len() > 200 << 20 {
                return Err("QIDI model XML exceeds 200 MiB".into());
            }
            let edited = set_qidi_application_metadata(&data)?;
            let options = entry.options();
            dst.start_file(name, options)
                .map_err(|e| format!("QIDI model ZIP output: {e}"))?;
            dst.write_all(&edited)
                .map_err(|e| format!("QIDI model ZIP write: {e}"))?;
            marked_qidi_model = true;
            continue;
        }
        if name == "Metadata/project_settings.config" && !preserved.is_empty() {
            let mut data = Vec::new();
            entry
                .by_ref()
                .take(MAX_XML_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|e| format!("project settings read: {e}"))?;
            if data.len() as u64 > MAX_XML_BYTES {
                return Err("project settings JSON exceeds size limit".to_owned());
            }
            let modified = mark_process_overrides(&data, preserved)?;
            let options = entry.options();
            dst.start_file(name, options)
                .map_err(|e| format!("ZIP output: {e}"))?;
            dst.write_all(&modified)
                .map_err(|e| format!("ZIP write: {e}"))?;
            continue;
        }
        if name == "[Content_Types].xml" || name.ends_with(".rels") {
            let mut data = Vec::new();
            entry
                .by_ref()
                .take(MAX_XML_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|e| format!("{name}: {e}"))?;
            if data.len() as u64 > MAX_XML_BYTES {
                return Err(format!("{name}: OPC XML exceeds size limit"));
            }
            if let Some(rewritten) = rewrite_cache_references(&data)? {
                let options = entry.options();
                dst.start_file(name, options)
                    .map_err(|e| format!("ZIP output: {e}"))?;
                dst.write_all(&rewritten)
                    .map_err(|e| format!("ZIP write: {e}"))?;
                continue;
            }
        }
        // Preserve original compressed data and metadata for unmodified entries.
        // A prior OPC read advances ZipFile; open a fresh entry for raw copying.
        drop(entry);
        let fresh = src.by_index(index).map_err(|e| e.to_string())?;
        dst.raw_copy_file(fresh)
            .map_err(|e| format!("ZIP raw copy: {e}"))?;
    }

    if qidi && !marked_qidi_model {
        return Err("QIDI conversion missing primary model for importer compatibility".into());
    }
    let result = dst.finish().map_err(|e| format!("ZIP finalize: {e}"))?;
    result.sync_all().map_err(|e| format!("ZIP sync: {e}"))?;
    drop(result);

    // On the same volume, hard_link is atomic and fails if target exists.
    fs::hard_link(&cleanup.0, destination)
        .map_err(|e| format!("destination not created (no overwrite): {e}"))?;
    Ok(removed)
}

/// Mirror Go's markProcessOverrides: carry existing process keys forward,
/// append the explicitly preserved ones, sort/deduplicate keys, and expand
/// different_settings_to_system to the number of filament slots plus two.
fn mark_process_overrides(raw: &[u8], preserved: &[String]) -> Result<Vec<u8>, String> {
    let mut config: Value =
        serde_json::from_slice(raw).map_err(|e| format!("project settings JSON: {e}"))?;
    let settings = config
        .as_object_mut()
        .ok_or_else(|| "project settings JSON must be an object".to_owned())?;

    let slots = settings
        .get("filament_colour")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .map_or(4usize, Vec::len);

    let mut differences = settings
        .remove("different_settings_to_system")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    if differences.len() < slots + 2 {
        differences.resize(slots + 2, Value::Null);
    }

    let mut marked = BTreeSet::new();
    if let Some(prior) = differences.first().and_then(Value::as_str) {
        for key in prior.split(';') {
            if !key.trim().is_empty() {
                // As in Go, retain whitespace in valid keys and only use trim
                // to reject empty entries.
                marked.insert(key.to_owned());
            }
        }
    }
    marked.extend(preserved.iter().cloned());
    differences[0] = Value::String(marked.into_iter().collect::<Vec<_>>().join(";"));

    for value in differences.iter_mut().skip(1) {
        if value.is_null() {
            *value = Value::String(String::new());
        }
    }
    settings.insert(
        "different_settings_to_system".to_owned(),
        Value::Array(differences),
    );
    serde_json::to_vec_pretty(&config)
        .map_err(|e| format!("project settings JSON serialization: {e}"))
}

fn possible_cache_reference(raw: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(raw).to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(index) = rest.find("plate_") {
        let tail = &rest[index + 6..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && (tail[digits..].starts_with(".gcode") || tail[digits..].starts_with(".json"))
        {
            return true;
        }
        rest = tail;
    }
    false
}

fn is_cache_reference(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    let without_parent = normalized.strip_prefix("../").unwrap_or(&normalized);
    let trimmed = without_parent.strip_prefix('/').unwrap_or(without_parent);
    let base = trimmed.rsplit('/').next().unwrap_or("");
    is_cache(trimmed) || is_cache(&format!("Metadata/{base}"))
}

fn should_remove(node: &BytesStart<'_>) -> Result<bool, String> {
    if !matches!(node.local_name().as_ref(), "Override" | "Relationship") {
        return Ok(false);
    }
    for attr in node.attributes() {
        let attr = attr.map_err(|e| format!("OPC attribute: {e}"))?;
        if matches!(attr.key.local_name().as_ref(), "Target" | "PartName") {
            let value = attr
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|e| format!("OPC attribute value: {e}"))?;
            if is_cache_reference(&value) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Return None when no relevant cache reference is present, preserving source
/// bytes exactly. A rewritten XML document keeps its namespace and non-cache nodes.
fn rewrite_cache_references(raw: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if !possible_cache_reference(raw) {
        return Ok(None);
    }
    let mut reader = Reader::from_reader(raw);
    let mut writer = Writer::new(Vec::new());
    let mut skip = 0usize;
    let mut did_remove = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("invalid OPC XML: {e}"))?;
        match event {
            Event::Start(start) => {
                if skip > 0 {
                    skip += 1;
                } else if should_remove(&start)? {
                    skip = 1;
                    did_remove = true;
                } else {
                    writer
                        .write_event(Event::Start(start))
                        .map_err(|e| format!("OPC write: {e}"))?;
                }
            }
            Event::Empty(start) => {
                if skip == 0 {
                    if should_remove(&start)? {
                        did_remove = true;
                    } else {
                        writer
                            .write_event(Event::Empty(start))
                            .map_err(|e| format!("OPC write: {e}"))?;
                    }
                }
            }
            Event::End(end) => {
                if skip > 0 {
                    skip -= 1;
                } else {
                    writer
                        .write_event(Event::End(end))
                        .map_err(|e| format!("OPC write: {e}"))?;
                }
            }
            Event::Eof => break,
            other if skip == 0 => writer
                .write_event(other)
                .map_err(|e| format!("OPC write: {e}"))?,
            _ => {}
        }
    }
    if did_remove {
        Ok(Some(writer.into_inner()))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn recognizes_exact_cache_reference_targets() {
        for target in [
            "Metadata/plate_1.gcode",
            "/Metadata/plate_2.json",
            "../Metadata/plate_3.gcode.md5",
            "plate_6.gcode",
            r"\Metadata\plate_7.gcode",
        ] {
            assert!(is_cache_reference(target), "{target}");
        }
        for target in [
            "Metadata/plate_1.png",
            "Metadata/plate_1_small.png",
            "Metadata/plate_6.GCODE",
            "Unrelated/foo.gcode",
        ] {
            assert!(!is_cache_reference(target), "{target}");
        }
    }

    #[test]
    fn strips_only_cache_references_and_preserves_xml_namespace() {
        let source = br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Target="/3D/3dmodel.model" Id="model"/>
<Relationship Target="/Metadata/plate_1.png" Id="preview"/>
<Relationship Target="/Metadata/plate_1.gcode" Id="cache"/>
</Relationships>"#;
        let output = rewrite_cache_references(source).unwrap().unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("http://schemas.openxmlformats.org/package/2006/relationships"));
        assert!(text.contains("plate_1.png"));
        assert!(text.contains("3dmodel.model"));
        assert!(!text.contains("plate_1.gcode"));
    }

    #[test]
    fn leaves_thumbnail_only_xml_byte_identical() {
        let source = br#"<Relationships xmlns="urn:test"><Relationship Target="plate_1.png"/></Relationships>"#;
        assert!(rewrite_cache_references(source).unwrap().is_none());
    }

    #[test]
    fn preserves_process_keys_and_respects_filament_slot_count() {
        let source = br##"{
            "printer_model": "Bambu Lab A1",
            "filament_colour": ["#123456", "#abcdef", "#808080"],
            "different_settings_to_system": ["existing;cooling;existing", null, 7]
        }"##;
        let result =
            mark_process_overrides(source, &["flow_ratio".to_owned(), "cooling".to_owned()])
                .unwrap();
        let value: Value = serde_json::from_slice(&result).unwrap();
        let settings = value["different_settings_to_system"].as_array().unwrap();
        assert_eq!(settings.len(), 5);
        assert_eq!(settings[0], "cooling;existing;flow_ratio");
        assert_eq!(settings[1], "");
        assert_eq!(settings[2], 7);
        assert_eq!(settings[3], "");
        assert_eq!(settings[4], "");
        assert_eq!(value["filament_colour"][2], "#808080");
        assert_eq!(value["printer_model"], "Bambu Lab A1");
    }

    #[test]
    fn missing_slots_and_settings_use_go_defaults() {
        let result = mark_process_overrides(
            br#"{"printer_model":"P1","different_settings_to_system":null}"#,
            &["height".to_owned()],
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&result).unwrap();
        let settings = value["different_settings_to_system"].as_array().unwrap();
        assert_eq!(settings.len(), 6);
        assert_eq!(settings[0], "height");
        assert!(settings.iter().skip(1).all(|value| value == ""));
    }

    #[test]
    fn invalid_settings_fail_without_panic() {
        for raw in [b"{".as_slice(), b"[]", b"null"] {
            assert!(mark_process_overrides(raw, &["flow".to_owned()]).is_err());
        }
    }
}
