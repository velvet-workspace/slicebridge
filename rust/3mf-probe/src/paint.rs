//! Experimental, pure Rust port of bl2std's recursive 3MF triangle-paint
//! bitstream remapper. No printer commands or source-file writes.
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::collections::BTreeMap;

const MAX_PAINT_HEX: usize = 4 << 20;
const MAX_MODEL_XML: usize = 8 << 20;
const MAX_RECURSION: usize = 96;
const MAX_NODES: usize = 1_000_000;

/// Remap each nonzero leaf state through an explicit 1-based filament mapping.
/// The hexadecimal encoding is parsed in reverse nibble order, least-significant
/// bit first, exactly like Go pkg/paint.Remap. Unmapped states stay unchanged.
pub fn remap_paint_hex(s: &str, mapping: &BTreeMap<u32, u32>) -> Result<String, String> {
    if s.is_empty() || s.len() > MAX_PAINT_HEX {
        return Err("paint hex is empty or exceeds the 4 MB input limit".into());
    }
    let mut bits = Vec::with_capacity(s.len().saturating_mul(4));
    for byte in s.bytes().rev() {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return Err(format!("paint: invalid hex byte 0x{byte:02x}")),
        };
        for shift in 0..4 {
            bits.push((nibble >> shift) & 1);
        }
    }
    let mut pos = 0usize;
    let mut rewritten = Vec::with_capacity(bits.len());
    let mut nodes = 0usize;
    remap_node(&bits, &mut pos, &mut rewritten, mapping, 0, &mut nodes)?;
    if pos != bits.len() {
        return Err(format!(
            "paint: {} trailing bits after decoding",
            bits.len() - pos
        ));
    }
    if rewritten.len() % 4 != 0 || rewritten.len() > MAX_PAINT_HEX.saturating_mul(4) {
        return Err("paint: rewritten bitstream is not a bounded number of nibbles".into());
    }

    let mut encoded = Vec::with_capacity(rewritten.len() / 4);
    for chunk in rewritten.as_chunks::<4>().0 {
        let nibble = chunk[0] | (chunk[1] << 1) | (chunk[2] << 2) | (chunk[3] << 3);
        encoded.push(b"0123456789ABCDEF"[nibble as usize]);
    }
    encoded.reverse();
    String::from_utf8(encoded).map_err(|e| e.to_string())
}

fn read_bits(bits: &[u8], pos: &mut usize, count: usize) -> Result<u32, String> {
    if bits.len().saturating_sub(*pos) < count {
        return Err("paint: truncated bitstream".into());
    }
    let mut value = 0u32;
    for bit in 0..count {
        value |= u32::from(bits[*pos + bit]) << bit;
    }
    *pos += count;
    Ok(value)
}

fn write_bits(output: &mut Vec<u8>, mut value: u32, count: usize) {
    for _ in 0..count {
        output.push((value & 1) as u8);
        value >>= 1;
    }
}

fn remap_node(
    bits: &[u8],
    pos: &mut usize,
    output: &mut Vec<u8>,
    mapping: &BTreeMap<u32, u32>,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    if depth > MAX_RECURSION || *nodes >= MAX_NODES {
        return Err("paint: recursive tree exceeds depth or node limit".into());
    }
    *nodes += 1;
    let split_sides = read_bits(bits, pos, 2)?;
    write_bits(output, split_sides, 2);

    if split_sides == 0 {
        let mut state = read_bits(bits, pos, 2)?;
        if state == 3 {
            loop {
                let extension = read_bits(bits, pos, 4)?;
                state = state
                    .checked_add(extension)
                    .ok_or_else(|| "paint: state integer overflow".to_owned())?;
                if extension != 15 {
                    break;
                }
            }
        }
        if state > 0 {
            if let Some(mapped) = mapping.get(&state) {
                if *mapped > 0 {
                    state = *mapped;
                }
            }
        }
        if state < 3 {
            write_bits(output, state, 2);
        } else {
            write_bits(output, 3, 2);
            let mut extension = state - 3;
            while extension >= 15 {
                write_bits(output, 15, 4);
                extension -= 15;
            }
            write_bits(output, extension, 4);
        }
        return Ok(());
    }

    let special_side = read_bits(bits, pos, 2)?;
    write_bits(output, special_side, 2);
    for _ in 0..(split_sides + 1) {
        remap_node(bits, pos, output, mapping, depth + 1, nodes)?;
    }
    Ok(())
}

fn rewrite_triangle(
    start: &BytesStart<'_>,
    mapping: &BTreeMap<u32, u32>,
) -> Result<BytesStart<'static>, String> {
    let mut rewritten = start.to_owned();
    rewritten.clear_attributes();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("triangle paint XML attribute: {e}"))?;
        let key = attr.key.as_ref();
        if matches!(
            attr.key.local_name().as_ref(),
            "paint_color" | "mmu_segmentation"
        ) && !attr.value.is_empty()
        {
            let original = attr.value.as_ref();
            let updated = remap_paint_hex(original, mapping)?;
            rewritten.push_attribute((key, updated.as_str()));
        } else {
            // Preserve raw attribute bytes (including entity escaping) exactly.
            rewritten.push_attribute(attr);
        }
    }
    Ok(rewritten.into_owned())
}

/// Rewrite paint attributes on all triangles in 3MF model XML. Everything
/// else (geometry, namespaced markup, comments) passes through the XML writer.
/// The caller must validate the destination 3MF container before publishing it.
pub fn rewrite_model_paint_xml(
    raw: &[u8],
    mapping: &BTreeMap<u32, u32>,
) -> Result<Vec<u8>, String> {
    if raw.is_empty() || raw.len() > MAX_MODEL_XML {
        return Err("model XML is empty or exceeds 8 MB".into());
    }
    let mut reader = Reader::from_reader(raw);
    let mut writer = Writer::new(Vec::with_capacity(raw.len()));
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("invalid model XML: {e}"))?;
        match event {
            Event::Start(start) if start.local_name().as_ref() == "triangle" => {
                writer
                    .write_event(Event::Start(rewrite_triangle(&start, mapping)?))
                    .map_err(|e| format!("model XML output: {e}"))?;
            }
            Event::Empty(start) if start.local_name().as_ref() == "triangle" => {
                writer
                    .write_event(Event::Empty(rewrite_triangle(&start, mapping)?))
                    .map_err(|e| format!("model XML output: {e}"))?;
            }
            Event::Eof => break,
            other => writer
                .write_event(other)
                .map_err(|e| format!("model XML output: {e}"))?,
        }
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn simple_states_and_extended_state_match_go() {
        let simple = [
            ("4", 1, "4"),
            ("8", 2, "8"),
            ("0C", 3, "0C"),
            ("1C", 4, "1C"),
            ("2C", 5, "2C"),
            ("3C", 6, "3C"),
            ("DC", 16, "DC"),
        ];
        for (paint, source, expected) in simple {
            let mut mapping = BTreeMap::new();
            mapping.insert(source, source);
            assert_eq!(remap_paint_hex(paint, &mapping).unwrap(), expected);
        }
        let mapping = BTreeMap::from([(1, 2), (2, 1), (3, 1)]);
        assert_eq!(remap_paint_hex("4", &mapping).unwrap(), "8");
        assert_eq!(remap_paint_hex("8", &mapping).unwrap(), "4");
        assert_eq!(remap_paint_hex("0C", &mapping).unwrap(), "4");
        let original = remap_paint_hex("4882", &BTreeMap::new()).unwrap();
        assert_eq!(original, "4882");
        let changed = remap_paint_hex("4882", &mapping).unwrap();
        assert_ne!(changed, "4882");
    }

    #[test]
    fn invalid_and_truncated_paint_are_rejected() {
        assert!(remap_paint_hex("", &BTreeMap::new()).is_err());
        assert!(remap_paint_hex("X", &BTreeMap::new()).is_err());
        assert!(remap_paint_hex("4F", &BTreeMap::new()).is_err());
        assert!(remap_paint_hex("C", &BTreeMap::new()).is_err());
    }

    #[test]
    fn rewrites_only_triangle_paint_and_keeps_geometry() {
        let xml = br#"<model xmlns="urn:core"><triangles><triangle v1="0" paint_color="4" mmu_segmentation="8"/><triangle v1="1" paint_color="0C"/></triangles><meta paint_color="4"/></model>"#;
        let map = BTreeMap::from([(1, 2), (2, 1)]);
        let written = rewrite_model_paint_xml(xml, &map).unwrap();
        let result = std::str::from_utf8(&written).unwrap();
        assert!(result.contains("v1=\"0\""));
        assert!(result.contains("paint_color=\"8\""));
        assert!(result.contains("mmu_segmentation=\"4\""));
        assert!(result.contains("paint_color=\"0C\""));
        assert!(result.contains("<meta paint_color=\"4\""));
    }
}
