//! Experimental Rust ports of two isolated Go 3MF operations: inspection
//! and opt-in removal of cached slice artifacts into a new output file.
//! No printer-profile conversion or physical printing occurs.
//! Production SliceBridge continues using its Go engine.
mod archive_convert;
mod materials;
mod modelsettings;
mod paint;
mod projectsettings;
mod sanitize;
mod sliceinfo;
pub use archive_convert::convert_archive_experimental;
pub use materials::{map_native_material_settings, source_slot_plan};
pub use modelsettings::rewrite_model_settings;
pub use paint::{remap_paint_hex, rewrite_model_paint_xml};
pub use projectsettings::compose_project_settings;
pub use sanitize::{strip_slice_cache, strip_slice_cache_preserving};
pub use sliceinfo::{rewrite_slice_info_xml, Slot as SliceInfoSlot};

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use zip::ZipArchive;

pub(crate) const MAX_COMPRESSED_BYTES: u64 = 250 << 20;
const MAX_UNCOMPRESSED_BYTES: u64 = 1024 << 20;
const MAX_ENTRIES: usize = 10_000;
pub(crate) const MAX_MODEL_XML_BYTES: u64 = 200 << 20;
const MAX_SETTINGS_BYTES: u64 = 8 << 20;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PackageFacts {
    pub vertices: u64,
    pub triangles: u64,
    pub slice_caches: u64,
    pub printer_model: String,
}

/// Read the same high-level fields as Go's inspectPackage without writing files.
/// Invalid, oversized or unsafe containers fail before any conversion is attempted.
pub fn inspect_package(path: impl AsRef<Path>) -> Result<PackageFacts, String> {
    let path = path.as_ref();
    let meta = fs::metadata(path).map_err(|e| format!("cannot stat 3MF: {e}"))?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_COMPRESSED_BYTES {
        return Err("3MF must be a nonempty regular file up to 250 MB".into());
    }
    let file = File::open(path).map_err(|e| format!("cannot open 3MF: {e}"))?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("invalid 3MF ZIP: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("archive contains too many entries".into());
    }
    // zip 2.x indexes entries by name and silently collapses duplicates.
    // Walk raw central-directory records so we retain Go's reject-on-duplicate rule.
    verify_central_directory(path, zip.central_directory_start(), zip.len())?;

    let mut facts = PackageFacts::default();
    let mut total = 0u64;
    let mut has_model = false;
    let mut has_content_types = false;
    let mut seen = HashSet::with_capacity(zip.len());

    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|e| format!("ZIP entry: {e}"))?;
        let name = entry.name().to_owned();
        if !safe_entry_name(&name) {
            return Err(format!("unsafe archive entry {name:?}"));
        }
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate archive entry {name:?}"));
        }
        total = total
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_UNCOMPRESSED_BYTES)
            .ok_or_else(|| "archive exceeds decompressed size limit".to_string())?;

        if name == "[Content_Types].xml" {
            has_content_types = true;
        }
        if is_cache(&name) {
            facts.slice_caches += 1;
        }

        if name.starts_with("3D/") && name.to_ascii_lowercase().ends_with(".model") {
            // Reject via the central-directory size *before* asking the XML
            // reader to materialize a huge text/comment token. The old
            // take(200MiB) limit could consume 200 MiB and end mid-XML.
            // The complete archive converter independently enforces this
            // same bound while auditing and rewriting model parts.
            if entry.size() > MAX_MODEL_XML_BYTES {
                return Err(format!("{name}: model XML exceeds 200 MiB limit"));
            }
            has_model = true;
            let mut xml = Reader::from_reader(BufReader::new(entry.take(MAX_MODEL_XML_BYTES)));
            let mut buf = Vec::new();
            loop {
                match xml.read_event_into(&mut buf) {
                    Ok(Event::Start(node) | Event::Empty(node)) => match node.local_name().as_ref()
                    {
                        "vertex" => facts.vertices += 1,
                        "triangle" => facts.triangles += 1,
                        _ => {}
                    },
                    Ok(Event::Eof) => break,
                    Err(e) => return Err(format!("invalid 3D XML: {e}")),
                    _ => {}
                }
                buf.clear();
            }
        } else if name == "Metadata/project_settings.config" {
            let mut data = Vec::new();
            entry
                .take(MAX_SETTINGS_BYTES)
                .read_to_end(&mut data)
                .map_err(|e| format!("project settings read: {e}"))?;
            let settings: serde_json::Value =
                serde_json::from_slice(&data).map_err(|e| format!("project settings JSON: {e}"))?;
            match settings {
                serde_json::Value::Object(ref obj) => {
                    facts.printer_model = obj
                        .get("printer_model")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .or_else(|| obj.get("printer_settings_id").and_then(|v| v.as_str()))
                        .unwrap_or_default()
                        .to_owned();
                }
                serde_json::Value::Null => {} // Go accepts JSON null into a nil map.
                _ => return Err("project settings JSON: expected object".into()),
            }
        }
    }

    if !has_model || facts.vertices == 0 || facts.triangles == 0 {
        return Err(
            "editable 3D mesh geometry is required (sliced-only 3MF is unsupported)".into(),
        );
    }
    if !has_content_types {
        return Err("missing [Content_Types].xml".into());
    }
    Ok(facts)
}

/// Reject duplicate raw names before ZipArchive's keyed index can collapse them.
/// Central-directory offsets come from the ZIP parser; every header is bounded.
fn verify_central_directory(
    path: &Path,
    offset: u64,
    visible_entries: usize,
) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("central directory open: {e}"))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("central directory seek: {e}"))?;

    let mut raw_names = HashSet::new();
    let mut scanned = 0usize;
    loop {
        let mut signature = [0u8; 4];
        file.read_exact(&mut signature)
            .map_err(|e| format!("central directory header: {e}"))?;
        if signature != [0x50, 0x4b, 0x01, 0x02] {
            break; // ZIP64 end record, central-directory signature, or EOCD.
        }
        scanned += 1;
        if scanned > MAX_ENTRIES {
            return Err("archive contains too many entries".into());
        }
        let mut fixed = [0u8; 42];
        file.read_exact(&mut fixed)
            .map_err(|e| format!("central directory entry: {e}"))?;
        let field = |i: usize| u16::from_le_bytes([fixed[i], fixed[i + 1]]) as usize;
        let name_len = field(24);
        let extra_len = field(26);
        let comment_len = field(28);
        let mut name = vec![0u8; name_len];
        file.read_exact(&mut name)
            .map_err(|e| format!("central directory name: {e}"))?;
        if !raw_names.insert(name.clone()) {
            return Err(format!(
                "duplicate archive entry {:?}",
                String::from_utf8_lossy(&name)
            ));
        }
        file.seek(SeekFrom::Current((extra_len + comment_len) as i64))
            .map_err(|e| format!("central directory fields: {e}"))?;
    }
    if scanned != visible_entries {
        return Err("ZIP central directory entries differ from archive index".into());
    }
    Ok(())
}

// Like Go's path.Clean safety check; additionally reject standalone "..".
fn safe_entry_name(name: &str) -> bool {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    !trimmed.is_empty()
        && !trimmed.starts_with('/')
        && !trimmed.contains('\\')
        && !trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

// Matching Go's Metadata/plate_[digits].(gcode|gcode.md5|json).
fn is_cache(name: &str) -> bool {
    let Some(tail) = name.strip_prefix("Metadata/plate_") else {
        return false;
    };
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && matches!(&tail[digits..], ".gcode" | ".gcode.md5" | ".json")
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn path_hardening() {
        for name in [
            "/3D/a.model",
            "../a",
            "a/../b",
            "a//b",
            "a/./b",
            "a\\b",
            "..",
            "",
        ] {
            assert!(!safe_entry_name(name), "{name:?}");
        }
        for name in [
            "3D/3dmodel.model",
            "[Content_Types].xml",
            "Metadata/",
            "a/b/c",
        ] {
            assert!(safe_entry_name(name), "{name:?}");
        }
    }

    #[test]
    fn only_exact_slice_cache_names_count() {
        for name in [
            "Metadata/plate_1.gcode",
            "Metadata/plate_123.gcode.md5",
            "Metadata/plate_5.json",
        ] {
            assert!(is_cache(name), "{name}");
        }
        for name in [
            "Metadata/plate_1.png",
            "Metadata/plate_.json",
            "metadata/plate_2.json",
            "Metadata/plate_7.gcode.bak",
        ] {
            assert!(!is_cache(name), "{name}");
        }
    }
}
