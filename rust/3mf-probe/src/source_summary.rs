//! Read-only source plate/material inspection matching the pinned Go
//! third_party/bl2std/pkg/converter/inspect.go contract. This is UI metadata,
//! not conversion planning or OEM authorization.
use crate::inspect_package;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::{result::ZipError, ZipArchive};

const MAX_METADATA_BYTES: u64 = 8 << 20;

#[derive(Default)]
struct Filament {
    id: u32,
    color: String,
    material: String,
    used_m: f64,
    used_g: f64,
}

fn attr(start: &BytesStart<'_>, name: &str) -> Result<Option<String>, String> {
    for field in start.attributes() {
        let field = field.map_err(|e| format!("source metadata attribute: {e}"))?;
        if field.key.local_name().as_ref() == name {
            return Ok(Some(
                field
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|e| format!("source metadata attribute {name}: {e}"))?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

// Go NormalizeColor: trim spaces / leading '#', discard alpha,
// and use white only when the source color itself cannot be parsed.
fn normalize_color(raw: &str) -> String {
    let mut c = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if c.len() == 8 {
        c = &c[..6];
    }
    if c.len() != 6 || !c.bytes().all(|b| b.is_ascii_hexdigit()) {
        return "#FFFFFF".into();
    }
    format!("#{}", c.to_ascii_uppercase())
}

fn add_filament(
    start: &BytesStart<'_>,
    filaments: &mut BTreeMap<u32, Filament>,
) -> Result<(), String> {
    let Some(id) = attr(start, "id")?.and_then(|id| id.parse::<u32>().ok()) else {
        return Ok(());
    };
    if id == 0 {
        return Ok(());
    }
    let material = attr(start, "type")?.filter(|value| !value.is_empty());
    let color = attr(start, "color")?.unwrap_or_default();
    let used_m = attr(start, "used_m")?
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    let used_g = attr(start, "used_g")?
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    let entry = filaments.entry(id).or_insert_with(|| Filament {
        id,
        color: normalize_color(&color),
        material: material.unwrap_or_else(|| "PLA".into()),
        used_m: 0.0,
        used_g: 0.0,
    });
    entry.used_m += used_m;
    entry.used_g += used_g;
    Ok(())
}

fn read_optional_part(
    archive: &mut ZipArchive<File>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(err) => return Err(format!("source metadata {name}: {err}")),
    };
    if entry.size() > MAX_METADATA_BYTES {
        return Err(format!("source metadata {name} exceeds 8 MB"));
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("source metadata {name} read: {e}"))?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(format!("source metadata {name} exceeds 8 MB"));
    }
    Ok(Some(bytes))
}

fn parse_slice_info(
    xml: &[u8],
    filaments: &mut BTreeMap<u32, Filament>,
) -> Result<(u64, String), String> {
    let mut reader = Reader::from_reader(xml);
    let mut depth = 0usize;
    let mut plate_depth = None;
    let mut plates = 0u64;
    let mut printer_model_id = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("Metadata/slice_info.config XML: {e}"))?;
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                let key = start.local_name();
                let name = key.as_ref();
                if depth == 1 && name == "plate" {
                    plates += 1;
                    if matches!(&event, Event::Start(_)) {
                        plate_depth = Some(depth + 1);
                    }
                } else if name == "filament" && (depth == 1 || plate_depth == Some(depth)) {
                    add_filament(start, filaments)?;
                } else if name == "metadata"
                    && plate_depth == Some(depth)
                    && printer_model_id.is_empty()
                    && attr(start, "key")?.as_deref() == Some("printer_model_id")
                {
                    printer_model_id = attr(start, "value")?.unwrap_or_default();
                }
                if matches!(&event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return Err("unbalanced source slice_info XML".into());
                }
                if plate_depth == Some(depth) {
                    plate_depth = None;
                }
                depth -= 1;
            }
            Event::DocType(_) => return Err("source slice_info DOCTYPE disallowed".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((plates, printer_model_id))
}

fn fallback_settings(
    archive: &mut ZipArchive<File>,
    filaments: &mut BTreeMap<u32, Filament>,
) -> Result<(), String> {
    let raw = read_optional_part(archive, "Metadata/project_settings.config")?
        .ok_or("source 3MF contains no filament information")?;
    let settings: Value =
        serde_json::from_slice(&raw).map_err(|e| format!("source project_settings JSON: {e}"))?;
    let colors = settings["filament_colour"]
        .as_array()
        .ok_or("source 3MF contains no filament information")?;
    let types = match settings.get("filament_type") {
        Some(Value::Null) | None => &[],
        Some(v) => v
            .as_array()
            .ok_or("source filament_type must be an array")?
            .as_slice(),
    };
    for (i, color) in colors.iter().enumerate() {
        let color = color
            .as_str()
            .ok_or("source filament_colour must contain strings")?;
        let material = types
            .get(i)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .unwrap_or("PLA");
        filaments.insert(
            (i + 1) as u32,
            Filament {
                id: (i + 1) as u32,
                color: normalize_color(color),
                material: material.into(),
                used_m: 0.0,
                used_g: 0.0,
            },
        );
    }
    Ok(())
}

/// Go-compatible plate count and sorted material/color/usage overview.
/// Validates source ZIP and editable geometry but never writes to it.
pub fn inspect_source_summary(path: &Path) -> Result<Value, String> {
    inspect_package(path)?;
    let file = File::open(path).map_err(|e| format!("source open: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("source ZIP: {e}"))?;
    let mut filaments = BTreeMap::new();
    let (plates, printer_model_id) =
        if let Some(xml) = read_optional_part(&mut archive, "Metadata/slice_info.config")? {
            parse_slice_info(&xml, &mut filaments)?
        } else {
            (0, String::new())
        };
    if filaments.is_empty() {
        fallback_settings(&mut archive, &mut filaments)?;
    }
    if filaments.is_empty() {
        return Err("source 3MF contains no filament information".into());
    }
    let filaments = filaments
        .into_values()
        .map(|filament| {
            json!({
                "id": filament.id,
                "color": filament.color,
                "type": filament.material,
                "used_m": filament.used_m,
                "used_g": filament.used_g,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "plates": plates,
        "filaments": filaments,
        "printer_model_id": printer_model_id,
    }))
}
