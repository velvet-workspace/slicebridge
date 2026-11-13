//! Experimental, in-memory port of Go `converter/sliceinfo.go`
//! (`rewriteSliceInfo`, `collectSliceUsage`, `aggregateUsage`,
//! `emitSlotFilaments`, `retargetPrinterModel`).
//!
//! Replaces the original `<filament>` entries of Metadata/slice_info.config
//! with exactly one entry per target slot and retargets the
//! `printer_model_id` metadata. Nothing is read from or written to disk.
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer, XmlVersion};
use std::collections::BTreeMap;

const MAX_SLICE_INFO_XML: usize = 8 << 20;
const MAX_SLOTS: usize = 64;

/// Target filament slot (Go `Slot`; only the attributes written to slice_info).
pub struct Slot {
    pub color: String,
    pub material: String,
}

/// Used metres and grams per target slot (Go `filamentUsage`).
type Usage = BTreeMap<i32, (f64, f64)>;

struct Collected {
    plates: Vec<Usage>,
    root: Usage,
}

fn out_err<E: std::fmt::Display>(e: E) -> String {
    format!("slice_info XML output: {e}")
}

fn start_local(start: &BytesStart<'_>) -> String {
    let name = start.local_name();
    let local: &str = name.as_ref();
    local.to_owned()
}

fn end_local(end: &BytesEnd<'_>) -> String {
    let name = end.local_name();
    let local: &str = name.as_ref();
    local.to_owned()
}

fn attr_local(attr: &Attribute<'_>) -> String {
    let name = attr.key.local_name();
    let local: &str = name.as_ref();
    local.to_owned()
}

fn attr_value(attr: &Attribute<'_>) -> Result<String, String> {
    attr.normalized_value(XmlVersion::Implicit1_0)
        .map(|value| value.into_owned())
        .map_err(|e| format!("slice_info attribute value: {e}"))
}

/// XML 1.0 `Char` production; rejects text that could never be serialized.
fn is_xml_text(value: &str) -> bool {
    value.chars().all(|c| {
        matches!(c, '\t' | '\n' | '\r') || (c >= '\u{20}' && c != '\u{FFFE}' && c != '\u{FFFF}')
    })
}

/// Go `aggregateUsage` for one `<filament>`: unparsable ids, unmapped ids and
/// unparsable (or non-finite) numbers are skipped silently, as in Go.
fn add_filament(
    usage: &mut Usage,
    start: &BytesStart<'_>,
    mapping: &BTreeMap<i32, i32>,
) -> Result<(), String> {
    let (mut id, mut used_m, mut used_g) = (None, None, None);
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("slice_info filament attribute: {e}"))?;
        match attr_local(&attr).as_str() {
            "id" => id = Some(attr_value(&attr)?),
            "used_m" => used_m = Some(attr_value(&attr)?),
            "used_g" => used_g = Some(attr_value(&attr)?),
            _ => {}
        }
    }
    let Some(id) = id.and_then(|v| v.parse::<i32>().ok()) else {
        return Ok(());
    };
    let Some(slot) = mapping.get(&id).copied() else {
        return Ok(());
    };
    let entry = usage.entry(slot).or_insert((0.0, 0.0));
    let parse = |v: Option<String>| {
        v.and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite())
    };
    if let Some(v) = parse(used_m) {
        entry.0 += v;
    }
    if let Some(v) = parse(used_g) {
        entry.1 += v;
    }
    if !entry.0.is_finite() || !entry.1.is_finite() {
        return Err("slice_info filament usage overflow".to_owned());
    }
    Ok(())
}

/// Go `collectSliceUsage`: per direct-child `<plate>` filaments and the
/// root-level filaments, with the root element's children as the scope.
fn collect_usage(raw: &[u8], mapping: &BTreeMap<i32, i32>) -> Result<Collected, String> {
    let mut reader = Reader::from_reader(raw);
    let mut collected = Collected {
        plates: Vec::new(),
        root: Usage::new(),
    };
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut current_plate: Option<usize> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("slice_info XML: {e}"))?;
        let (start, is_start) = match event {
            Event::Start(start) => (start, true),
            Event::Empty(start) => (start, false),
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "slice_info XML: unbalanced end tag".to_owned())?;
                continue;
            }
            Event::DocType(_) => return Err("slice_info XML: DOCTYPE is not allowed".to_owned()),
            Event::Eof => break,
            _ => continue,
        };
        let level = depth + 1;
        if level == 1 {
            roots += 1;
            if roots > 1 {
                return Err("slice_info XML: multiple root elements".to_owned());
            }
        } else if level == 2 {
            current_plate = None;
            match start_local(&start).as_str() {
                "plate" => {
                    collected.plates.push(Usage::new());
                    current_plate = Some(collected.plates.len() - 1);
                }
                "filament" => add_filament(&mut collected.root, &start, mapping)?,
                _ => {}
            }
        } else if level == 3 && start_local(&start) == "filament" {
            if let Some(index) = current_plate {
                add_filament(&mut collected.plates[index], &start, mapping)?;
            }
        }
        if is_start {
            depth += 1;
        }
    }
    if depth != 0 {
        return Err("slice_info XML: unclosed element".to_owned());
    }
    if roots == 0 {
        return Err("slice_info XML: no root element".to_owned());
    }
    Ok(collected)
}

/// Go `emitSlotFilaments`: indentation, one `<filament .../>` per slot, newline.
fn emit_slot_filaments(
    writer: &mut Writer<Vec<u8>>,
    slots: &[Slot],
    usage: &Usage,
) -> Result<(), String> {
    for (index, slot) in slots.iter().enumerate() {
        let (used_m, used_g) = usage
            .get(&(index as i32 + 1))
            .copied()
            .unwrap_or((0.0, 0.0));
        let id = (index + 1).to_string();
        let used_m = format!("{used_m:.2}");
        let used_g = format!("{used_g:.2}");
        let mut element = BytesStart::new("filament");
        element.push_attribute(("id", id.as_str()));
        element.push_attribute(("type", slot.material.as_str()));
        element.push_attribute(("color", slot.color.as_str()));
        element.push_attribute(("used_m", used_m.as_str()));
        element.push_attribute(("used_g", used_g.as_str()));
        let end = element.to_end().into_owned();
        writer
            .write_event(Event::Text(BytesText::new("  ")))
            .map_err(out_err)?;
        writer.write_event(Event::Start(element)).map_err(out_err)?;
        writer.write_event(Event::End(end)).map_err(out_err)?;
        writer
            .write_event(Event::Text(BytesText::new("\n")))
            .map_err(out_err)?;
    }
    Ok(())
}

/// Go `retargetPrinterModel`: `<metadata key="printer_model_id" value=..>`
/// gets every `value` attribute replaced. Returns None when unchanged.
fn retarget_printer_model(
    start: &BytesStart<'_>,
    model_id: &str,
) -> Result<Option<BytesStart<'static>>, String> {
    if model_id.is_empty() || start_local(start) != "metadata" {
        return Ok(None);
    }
    let mut is_model_key = false;
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("slice_info metadata attribute: {e}"))?;
        if attr_local(&attr) == "key" && attr_value(&attr)? == "printer_model_id" {
            is_model_key = true;
        }
    }
    if !is_model_key {
        return Ok(None);
    }
    let mut rewritten = start.to_owned();
    rewritten.clear_attributes();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("slice_info metadata attribute: {e}"))?;
        if attr_local(&attr) == "value" {
            let key: &str = attr.key.as_ref();
            rewritten.push_attribute((key, model_id));
        } else {
            // Preserve raw attribute bytes (including entity escaping).
            rewritten.push_attribute(attr);
        }
    }
    Ok(Some(rewritten.into_owned()))
}

/// Go `usageForParent`.
fn usage_for_parent<'a>(parent: &str, plate_index: i64, collected: &'a Collected) -> &'a Usage {
    if parent == "plate" && plate_index >= 0 {
        if let Some(usage) = collected.plates.get(plate_index as usize) {
            return usage;
        }
    }
    &collected.root
}

fn validate(
    raw: &[u8],
    slots: &[Slot],
    mapping: &BTreeMap<i32, i32>,
    model_id: &str,
) -> Result<(), String> {
    if raw.is_empty() || raw.len() > MAX_SLICE_INFO_XML {
        return Err("slice_info XML is empty or exceeds 8 MiB".to_owned());
    }
    if slots.is_empty() || slots.len() > MAX_SLOTS {
        return Err(format!("slot count must be 1..={MAX_SLOTS}"));
    }
    for slot in slots {
        if !is_xml_text(&slot.color)
            || !is_xml_text(&slot.material)
            || slot.color.contains(['\n', '\r', '\t'])
            || slot.material.contains(['\n', '\r', '\t'])
        {
            return Err("slot color/material is unsafe inside XML attributes".to_owned());
        }
    }
    if !is_xml_text(model_id) || model_id.contains(['\n', '\r', '\t']) {
        return Err("model id contains characters invalid in XML".to_owned());
    }
    for (source, target) in mapping {
        if *target < 1 || *target as usize > slots.len() {
            return Err(format!(
                "mapping {source} -> {target} is outside slots 1..={}",
                slots.len()
            ));
        }
    }
    Ok(())
}

/// Rewrite Metadata/slice_info.config: drop every `<filament>`, append one per
/// target slot at the end of each `<plate>` (or of `<config>` when the file
/// has no direct-child plates) with usage summed through `mapping`
/// (source filament id -> 1-based target slot), and retarget
/// `printer_model_id` to `model_id` (empty keeps the original).
pub fn rewrite_slice_info_xml(
    raw: &[u8],
    slots: &[Slot],
    mapping: &BTreeMap<i32, i32>,
    model_id: &str,
) -> Result<Vec<u8>, String> {
    validate(raw, slots, mapping, model_id)?;
    let collected = collect_usage(raw, mapping)?;
    let parent = if collected.plates.is_empty() {
        "config"
    } else {
        "plate"
    };

    let mut reader = Reader::from_reader(raw);
    let mut writer = Writer::new(Vec::with_capacity(raw.len() + slots.len() * 96));
    let mut plate_index: i64 = -1;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("slice_info XML: {e}"))?;
        match event {
            Event::Start(start) => {
                let local = start_local(&start);
                if local == "filament" {
                    continue;
                }
                if local == "plate" {
                    plate_index += 1;
                }
                let start = match retarget_printer_model(&start, model_id)? {
                    Some(changed) => changed,
                    None => start.into_owned(),
                };
                writer.write_event(Event::Start(start)).map_err(out_err)?;
            }
            Event::Empty(start) => {
                let local = start_local(&start);
                if local == "filament" {
                    continue;
                }
                if local == "plate" {
                    plate_index += 1;
                }
                let start = match retarget_printer_model(&start, model_id)? {
                    Some(changed) => changed,
                    None => start.into_owned(),
                };
                if local == parent {
                    // Go sees Start+End for self-closing tags; expand so the
                    // filaments land inside the element.
                    let end = start.to_end().into_owned();
                    writer.write_event(Event::Start(start)).map_err(out_err)?;
                    let usage = usage_for_parent(parent, plate_index, &collected);
                    emit_slot_filaments(&mut writer, slots, usage)?;
                    writer.write_event(Event::End(end)).map_err(out_err)?;
                } else {
                    writer.write_event(Event::Empty(start)).map_err(out_err)?;
                }
            }
            Event::End(end) => {
                let local = end_local(&end);
                if local == "filament" {
                    continue;
                }
                if local == parent {
                    let usage = usage_for_parent(parent, plate_index, &collected);
                    emit_slot_filaments(&mut writer, slots, usage)?;
                }
                writer.write_event(Event::End(end)).map_err(out_err)?;
            }
            Event::Eof => break,
            other => writer.write_event(other).map_err(out_err)?,
        }
    }
    Ok(writer.into_inner())
}
