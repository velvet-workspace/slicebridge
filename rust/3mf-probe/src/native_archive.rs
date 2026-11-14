//! Experimental composition for native OEM archive conversion. A verified
//! request is planned entirely in memory, the request's filament palette is
//! cross-checked against the source slice info, and the existing fail-closed
//! archive converter performs the only write. Callers receive a compact receipt;
//! the OEM plan and its machine scripts never leave the process.
use crate::{build_native_conversion_plan, convert_archive_experimental};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

const MAX_SLICE_INFO_BYTES: u64 = 8 << 20;

/// Source filament ID to every distinct (material, color) declared for it.
pub(crate) type Palette = BTreeMap<u32, BTreeSet<(String, String)>>;

fn attribute(start: &BytesStart<'_>, name: &str) -> Result<Option<String>, String> {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| format!("source slice_info attribute: {e}"))?;
        if attribute.key.local_name().as_ref() == name {
            let decoded = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|e| format!("source slice_info attribute {name}: {e}"))?;
            return Ok(Some(decoded.into_owned()));
        }
    }
    Ok(None)
}

/// Reads every `<filament>` the source slice info declares, so the request
/// must describe this project's palette rather than a plausible one.
fn source_palette(source: &Path) -> Result<Palette, String> {
    let file = File::open(source).map_err(|e| format!("source open: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("input 3MF ZIP: {e}"))?;
    source_palette_from_archive(&mut archive)
}

// Must also run against the archive converter's OWN frozen source snapshot;
// checking an independently opened source path is vulnerable to source changes
// between request validation and snapshot capture.
pub(crate) fn source_palette_from_archive(
    archive: &mut ZipArchive<File>,
) -> Result<Palette, String> {
    let entry = archive
        .by_name("Metadata/slice_info.config")
        .map_err(|_| "input 3MF has no declared slice_info filament palette".to_owned())?;
    let mut xml = Vec::new();
    entry
        .take(MAX_SLICE_INFO_BYTES + 1)
        .read_to_end(&mut xml)
        .map_err(|e| format!("source slice_info read: {e}"))?;
    if xml.len() as u64 > MAX_SLICE_INFO_BYTES {
        return Err("source slice_info exceeds 8 MB limit".into());
    }
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut palette = Palette::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("source slice_info XML: {e}"))?
        {
            Event::Start(ref start) | Event::Empty(ref start)
                if start.local_name().as_ref() == "filament" =>
            {
                let Some(id) = attribute(start, "id")?.and_then(|v| v.parse::<u32>().ok()) else {
                    continue;
                };
                if id == 0 {
                    continue;
                }
                let material = attribute(start, "type")?.unwrap_or_default();
                let color = attribute(start, "color")?.unwrap_or_default();
                palette.entry(id).or_default().insert((material, color));
            }
            Event::DocType(_) => return Err("source slice_info DOCTYPE disallowed".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if palette.is_empty() {
        return Err("input 3MF slice_info contains no positive filament IDs".into());
    }
    Ok(palette)
}

/// Every request filament must match exactly one declared source palette entry
/// (material exact, color case-insensitive). Mismatches would print the wrong
/// material or color, so they are refused before any output is created.
pub(crate) fn match_request_palette(request: &Value, palette: &Palette) -> Result<(), String> {
    let filaments = request
        .get("filaments")
        .and_then(Value::as_array)
        .ok_or_else(|| "filaments must be an array".to_owned())?;
    for filament in filaments {
        let id = filament
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(|| "filament ID must be a positive integer".to_owned())?;
        let color = filament
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let material = filament
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let declared = palette
            .get(&id)
            .ok_or_else(|| format!("request filament {id} is absent from source slice_info"))?;
        let matches = declared.len() == 1
            && declared.iter().all(|(source_type, source_color)| {
                source_type == material && source_color.eq_ignore_ascii_case(color)
            });
        if !matches {
            return Err(format!(
                "request filament {id} does not match source slice_info color/material"
            ));
        }
    }
    Ok(())
}

/// Converts a source 3MF using an explicit, verified OEM request.
///
/// Ordering is deliberate: refuse existing destinations, require an explicit
/// source settings snapshot, build the plan in memory (failing on any OEM or
/// material defect), cross-check the palette against the source, and only then
/// hand the plan to the archive converter. The converter re-validates the
/// source snapshot, settings, mapping and material references itself.
pub fn convert_native_archive(
    request: &Value,
    source: &Path,
    destination: &Path,
) -> Result<Value, String> {
    if destination.exists() {
        return Err("destination already exists; refusing overwrite".into());
    }
    if !request
        .get("source_settings")
        .and_then(Value::as_object)
        .is_some_and(|settings| !settings.is_empty())
    {
        return Err("source_settings snapshot must be a nonempty JSON object".into());
    }
    let mut plan = build_native_conversion_plan(request)?;
    match_request_palette(request, &source_palette(source)?)?;
    // The archive converter re-checks this exact material/color declaration
    // against its immutable captured ZIP, closing a TOCTOU gap.
    plan["source_filaments"] = request["filaments"].clone();
    let removed = convert_archive_experimental(source, destination, &plan)?;
    let mut receipt = json!({
        "ok": true,
        "experimental": true,
        "removed": removed,
        "planner": "rust-native",
    });
    // Existing CLI/JSONL users retain the compact receipt. The explicit
    // opt-in desktop host needs the actual Rust-owned mapping and preserved
    // settings for its existing conversion-report contract.
    if request
        .get("include_receipt_details")
        .and_then(Value::as_bool)
        == Some(true)
    {
        receipt["slots"] = plan["slots"].clone();
        receipt["mapping"] = plan["mapping"].clone();
        receipt["preserved"] = plan["preserved"].clone();
    }
    Ok(receipt)
}
