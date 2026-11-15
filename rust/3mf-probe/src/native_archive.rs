//! Experimental composition for native OEM archive conversion. A verified
//! request is planned entirely in memory, the request's filament palette is
//! cross-checked against the source slice info, and the existing fail-closed
//! archive converter performs the only write. Callers receive a compact receipt;
//! the OEM plan and its machine scripts never leave the process.
use crate::{build_native_conversion_plan, convert_archive_experimental, inspect_package};
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

// Go's pinned source inspector accepts a real unsliced Bambu 3MF whose
// slice_info XML declares no filament elements, provided the source project
// settings carry a complete material/color array. Validate that secondary
// palette directly from the actual ZIP, never from caller-supplied settings
// or invented machine defaults. Incomplete/invalid entries are refused.
fn fallback_project_palette(archive: &mut ZipArchive<File>) -> Result<Palette, String> {
    let entry = archive
        .by_name("Metadata/project_settings.config")
        .map_err(|_| "unsliced source has no project_settings filament palette".to_owned())?;
    let mut data = Vec::new();
    entry
        .take(MAX_SLICE_INFO_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("source project palette read: {e}"))?;
    if data.len() as u64 > MAX_SLICE_INFO_BYTES {
        return Err("source project palette exceeds 8 MB limit".into());
    }
    let settings: Value =
        serde_json::from_slice(&data).map_err(|e| format!("source project palette JSON: {e}"))?;
    project_palette_from_settings(&settings)
}

fn project_palette_from_settings(settings: &Value) -> Result<Palette, String> {
    let colors = settings
        .get("filament_colour")
        .and_then(Value::as_array)
        .ok_or_else(|| "unsliced source lacks filament_colour array".to_owned())?;
    let materials = settings
        .get("filament_type")
        .and_then(Value::as_array)
        .ok_or_else(|| "unsliced source lacks filament_type array".to_owned())?;
    if colors.is_empty() || colors.len() > 64 || colors.len() != materials.len() {
        return Err("unsliced source palette lengths must match and contain 1..64 entries".into());
    }
    let mut palette = Palette::new();
    for (index, (color, material)) in colors.iter().zip(materials).enumerate() {
        let raw = color
            .as_str()
            .ok_or_else(|| format!("unsliced source color {} must be a string", index + 1))?
            .trim()
            .trim_start_matches('#');
        if !(raw.len() == 6 || raw.len() == 8) || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "unsliced source color {} is not a verified RGB hex",
                index + 1
            ));
        }
        let material = material
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("unsliced source material {} is missing", index + 1))?;
        // Match the pinned Go inspector's hexadecimal uppercasing and
        // optional source alpha removal without inventing an invalid color.
        let color = format!("#{}", raw[..6].to_ascii_uppercase());
        palette
            .entry((index + 1) as u32)
            .or_default()
            .insert((material.to_owned(), color));
    }
    Ok(palette)
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
    let mut declared_any_filament = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("source slice_info XML: {e}"))?
        {
            Event::Start(ref start) | Event::Empty(ref start)
                if start.local_name().as_ref() == "filament" =>
            {
                declared_any_filament = true;
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
        if declared_any_filament {
            // A present but malformed/zero/undeclared filament is NOT an
            // unsliced empty palette. Refuse it rather than falling back.
            return Err("input 3MF slice_info contains no positive filament IDs".into());
        }
        return fallback_project_palette(archive);
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

/// Construct a Rust-owned conversion request from the ACTUAL source archive.
///
/// Unlike the legacy Go-driven request, the caller passes only the verified
/// installed OEM bundle; source colors, material IDs, and process settings
/// are not supplied by the caller or guessed from the target. This is a step
/// toward an independent Rust engine, not an OEM filesystem/profile resolver.
/// A complete profile bundle remains mandatory, and no installed file is
/// modified by this operation.
fn source_owned_native_request(bundle: &Value, source: &Path) -> Result<Value, String> {
    let file = File::open(source).map_err(|e| format!("source open: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("input 3MF ZIP: {e}"))?;
    let palette = source_palette_from_archive(&mut archive)?;
    if palette.is_empty() || palette.len() > 64 {
        return Err("source must declare between 1 and 64 material identities".into());
    }
    let mut filaments = Vec::with_capacity(palette.len());
    for (id, declarations) in palette {
        if id == 0 || id > i32::MAX as u32 || declarations.len() != 1 {
            return Err(format!("ambiguous or unsupported source filament ID {id}"));
        }
        let (material, color) = declarations.iter().next().expect("exactly one declaration");
        let color_hex = color.trim().strip_prefix('#').unwrap_or(color.trim());
        if material.trim().is_empty()
            || !(color_hex.len() == 6 || color_hex.len() == 8)
            || !color_hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(format!("source filament {id} lacks a valid material/color"));
        }
        filaments.push(json!({"id":id,"type":material,"color":color}));
    }

    let settings_entry = archive
        .by_name("Metadata/project_settings.config")
        .map_err(|_| "source lacks original project_settings.config".to_owned())?;
    let mut data = Vec::new();
    settings_entry
        .take(MAX_SLICE_INFO_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("source settings read: {e}"))?;
    if data.len() as u64 > MAX_SLICE_INFO_BYTES {
        return Err("source project settings exceed 8 MB limit".into());
    }
    let settings: Value =
        serde_json::from_slice(&data).map_err(|e| format!("source project settings JSON: {e}"))?;
    if !settings.as_object().is_some_and(|map| !map.is_empty()) {
        return Err("source project settings must be a nonempty JSON object".into());
    }
    // No trust in caller-supplied source metadata: the request is constructed
    // exclusively from the read-only source ZIP and verified OEM bundle.
    Ok(json!({
        "bundle": bundle,
        "filaments": filaments,
        "source_settings": settings,
        "include_receipt_details": true
    }))
}

/// Validate a previously resolved OEM bundle against the original source 3MF.
/// This performs the same source-owned palette discovery and complete Rust
/// conversion planning as autonomous conversion, without writing an output.
pub fn validate_native_autonomous_input(bundle: &Value, source: &Path) -> Result<Value, String> {
    inspect_package(source)?;
    let request = source_owned_native_request(bundle, source)?;
    let plan = build_native_conversion_plan(&request)?;
    Ok(json!({
        "target": bundle["target"],
        "printer": bundle["profile"]["display_name"],
        "slot_count": plan["slot_count"],
        "validated_source": true
    }))
}

/// Full native 3MF conversion with source-driven material discovery in Rust.
/// The caller must provide a complete, already-resolved installed OEM bundle
/// (not a source filament map, Go plan, guessed nozzle, or startup script).
/// The existing bounded converter independently recaptures the source bytes
/// and checks the same palette and settings against its own snapshot.
pub fn convert_native_autonomous(
    bundle: &Value,
    source: &Path,
    destination: &Path,
) -> Result<Value, String> {
    if destination.exists() {
        return Err("destination already exists; refusing overwrite".into());
    }
    // Validate archive entry names, duplicate names, expanded ZIP size, and
    // geometry bounds before even discovering source-owned print parameters.
    inspect_package(source)?;
    let request = source_owned_native_request(bundle, source)?;
    convert_native_archive(&request, source, destination)
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

#[cfg(test)]
mod unsliced_real_source_tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::write::{SimpleFileOptions, ZipWriter};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

    struct ScratchArchive(PathBuf);
    impl Drop for ScratchArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn input(slice_info: &str, settings: &Value) -> ScratchArchive {
        let name = format!(
            "slicebridge-unsliced-palette-{}-{}.3mf",
            std::process::id(),
            TEST_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default();
        writer
            .start_file("Metadata/slice_info.config", options)
            .unwrap();
        writer.write_all(slice_info.as_bytes()).unwrap();
        writer
            .start_file("Metadata/project_settings.config", options)
            .unwrap();
        writer.write_all(settings.to_string().as_bytes()).unwrap();
        writer.finish().unwrap();
        ScratchArchive(path)
    }

    #[test]
    fn real_unsliced_empty_slice_info_uses_only_verified_project_palette() {
        let original = json!({
            "filament_colour": ["#9B9EA0", "#12aBcD88"],
            "filament_type": ["PETG", "PLA"],
        });
        let file = input(
            "<?xml version=\"1.0\"?><config><header/></config>",
            &original,
        );
        let mut archive = ZipArchive::new(File::open(&file.0).unwrap()).unwrap();
        let palette = source_palette_from_archive(&mut archive).unwrap();
        assert_eq!(palette.len(), 2);
        assert!(palette[&1].contains(&("PETG".to_owned(), "#9B9EA0".to_owned())));
        assert!(palette[&2].contains(&("PLA".to_owned(), "#12ABCD".to_owned())));
        let good = json!({"filaments": [
            {"id":1,"type":"PETG","color":"#9B9EA0"},
            {"id":2,"type":"PLA","color":"#12ABCD"}
        ]});
        match_request_palette(&good, &palette).unwrap();
        let mut wrong = good.clone();
        wrong["filaments"][1]["color"] = json!("#FFFFFF");
        assert!(match_request_palette(&wrong, &palette).is_err());
        wrong = good.clone();
        wrong["filaments"][0]["type"] = json!("PLA");
        assert!(match_request_palette(&wrong, &palette).is_err());
    }

    #[test]
    fn malformed_or_zero_slice_palette_never_falls_back_to_project_settings() {
        let settings = json!({
            "filament_colour": ["#112233"],
            "filament_type": ["PETG"]
        });
        for xml in [
            r##"<config><plate><filament id="0" type="PETG" color="#112233"/></plate></config>"##,
            r##"<config><plate><filament id="garbled" type="PETG" color="#112233"/></plate></config>"##,
        ] {
            let file = input(xml, &settings);
            let mut archive = ZipArchive::new(File::open(&file.0).unwrap()).unwrap();
            let error = source_palette_from_archive(&mut archive).unwrap_err();
            assert!(error.contains("no positive filament IDs"), "{error}");
        }
    }

    #[test]
    fn project_settings_fallback_refuses_missing_or_guessed_materials_and_colors() {
        let valid = json!({"filament_colour":["#112233"],"filament_type":["PETG"]});
        assert!(project_palette_from_settings(&valid).is_ok());
        let bad = [
            json!({"filament_colour":["not-a-color"],"filament_type":["PETG"]}),
            json!({"filament_colour":["#112233"],"filament_type":[]}),
            json!({"filament_colour":["#112233"]}),
            json!({"filament_colour":["#112233"],"filament_type":[""]}),
            json!({"filament_colour":[],"filament_type":[]}),
            json!({"filament_colour":["#112233", "#445566"],"filament_type":["PLA"]}),
        ];
        for req in bad {
            assert!(
                project_palette_from_settings(&req).is_err(),
                "incomplete unsliced palette accepted: {req}"
            );
        }
    }

    #[test]
    fn populated_slice_info_cannot_be_overridden_by_foreign_project_settings() {
        let file = input(
            r##"<config><plate><filament id="1" type="PLA" color="#FF0000"/></plate></config>"##,
            &json!({"filament_colour":["#0000FF"],"filament_type":["ABS"]}),
        );
        let mut archive = ZipArchive::new(File::open(&file.0).unwrap()).unwrap();
        let palette = source_palette_from_archive(&mut archive).unwrap();
        assert!(palette[&1].contains(&("PLA".to_owned(), "#FF0000".to_owned())));
        let fake = json!({"filaments":[{"id":1,"type":"ABS","color":"#0000FF"}]});
        assert!(match_request_palette(&fake, &palette).is_err());
    }
}
