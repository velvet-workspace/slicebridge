#[path = "../src/sliceinfo.rs"]
mod sliceinfo;

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use sliceinfo::{rewrite_slice_info_xml, Slot};
use std::collections::BTreeMap;

type Attrs = Vec<(String, String)>;

struct Element {
    local: String,
    parent: String,
    /// 1-based count of `<plate>` start tags seen so far (0 before any).
    plate: usize,
    attrs: Attrs,
}

fn slot(color: &str, material: &str) -> Slot {
    Slot {
        color: color.to_owned(),
        material: material.to_owned(),
    }
}

fn two_slots() -> Vec<Slot> {
    vec![slot("#FF0000", "PLA"), slot("#00FF00", "PETG")]
}

fn identity(ids: &[i32]) -> BTreeMap<i32, i32> {
    ids.iter().map(|id| (*id, *id)).collect()
}

fn local_of(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or(raw).to_owned()
}

/// Parse with quick-xml (also asserting well-formedness) into a flat element list.
fn parse(xml: &[u8]) -> Vec<Element> {
    let mut reader = Reader::from_reader(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut plates = 0usize;
    let mut elements = Vec::new();
    loop {
        match reader.read_event().expect("output must be well-formed XML") {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let is_start = matches!(event, Event::Start(_));
                let (Event::Start(start) | Event::Empty(start)) = event else {
                    unreachable!()
                };
                let name = {
                    let q = start.name();
                    let s: &str = q.as_ref();
                    local_of(s)
                };
                if name == "plate" {
                    plates += 1;
                }
                let parent = stack.last().cloned().unwrap_or_default();
                let mut attrs = Attrs::new();
                for attr in start.attributes() {
                    let attr = attr.unwrap();
                    let key: &str = attr.key.as_ref();
                    let value = attr
                        .normalized_value(XmlVersion::Implicit1_0)
                        .unwrap()
                        .into_owned();
                    attrs.push((key.to_owned(), value));
                }
                if is_start {
                    stack.push(name.clone());
                }
                elements.push(Element {
                    local: name,
                    parent,
                    plate: plates,
                    attrs,
                });
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    elements
}

fn attr<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
    element
        .attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn named<'a>(elements: &'a [Element], local: &str) -> Vec<&'a Element> {
    elements.iter().filter(|e| e.local == local).collect()
}

fn run(xml: &str, slots: &[Slot], mapping: &BTreeMap<i32, i32>, model: &str) -> Vec<Element> {
    let out = rewrite_slice_info_xml(xml.as_bytes(), slots, mapping, model).unwrap();
    parse(&out)
}

fn usage_of(f: &Element) -> (&str, &str) {
    (attr(f, "used_m").unwrap(), attr(f, "used_g").unwrap())
}

const ONE_PLATE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <header><header_item key="X-BBL-Client-Type" value="slicer"/></header>
  <plate>
    <metadata key="index" value="1"/>
    <metadata key="printer_model_id" value="BL-P001"/>
    <object identify_id="10" name="cube" skipped="false"/>
    <filament id="1" tray_info_idx="GFA00" type="PLA" color="#FF0000" used_m="1.00" used_g="3.00"/>
    <filament id="2" type="PETG" color="#00FF00" used_m="2.50" used_g="7.50"/>
    <warning msg="careful" level="1"/>
  </plate>
</config>"##;

#[test]
fn one_plate_replaces_filaments_with_target_slots() {
    let out = run(
        ONE_PLATE,
        &[slot("#112233", "ABS"), slot("#445566", "TPU")],
        &identity(&[1, 2]),
        "",
    );
    let filaments = named(&out, "filament");
    assert_eq!(filaments.len(), 2);
    assert_eq!(filaments[0].parent, "plate");
    assert_eq!(attr(filaments[0], "id"), Some("1"));
    assert_eq!(attr(filaments[0], "type"), Some("ABS"));
    assert_eq!(attr(filaments[0], "color"), Some("#112233"));
    assert_eq!(usage_of(filaments[0]), ("1.00", "3.00"));
    assert_eq!(attr(filaments[1], "id"), Some("2"));
    assert_eq!(attr(filaments[1], "type"), Some("TPU"));
    assert_eq!(usage_of(filaments[1]), ("2.50", "7.50"));
    // Original-only attributes are gone.
    assert!(filaments.iter().all(|f| attr(f, "tray_info_idx").is_none()));
    // Unrelated content is preserved.
    assert_eq!(named(&out, "object").len(), 1);
    assert_eq!(named(&out, "warning").len(), 1);
    assert_eq!(named(&out, "header_item").len(), 1);
}

#[test]
fn two_plates_keep_separate_usage() {
    let xml = r#"<config>
<plate><metadata key="index" value="1"/>
<filament id="1" used_m="1.0" used_g="2.0"/></plate>
<plate><metadata key="index" value="2"/>
<filament id="1" used_m="4.0" used_g="5.0"/><filament id="2" used_m="2.0" used_g="6.0"/></plate>
</config>"#;
    let out = run(xml, &two_slots(), &identity(&[1, 2]), "");
    let filaments = named(&out, "filament");
    assert_eq!(filaments.len(), 4);
    let first: Vec<_> = filaments.iter().filter(|f| f.plate == 1).collect();
    let second: Vec<_> = filaments.iter().filter(|f| f.plate == 2).collect();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert_eq!(usage_of(first[0]), ("1.00", "2.00"));
    assert_eq!(usage_of(first[1]), ("0.00", "0.00"));
    assert_eq!(usage_of(second[0]), ("4.00", "5.00"));
    assert_eq!(usage_of(second[1]), ("2.00", "6.00"));
}

#[test]
fn root_level_filaments_without_plates_land_under_config() {
    let xml = r#"<config>
  <metadata key="printer_model_id" value="OLD"/>
  <filament id="1" used_m="1.5" used_g="2.25"/>
</config>"#;
    let out = run(xml, &two_slots(), &identity(&[1]), "NEW");
    let filaments = named(&out, "filament");
    assert_eq!(filaments.len(), 2);
    assert!(filaments.iter().all(|f| f.parent == "config"));
    assert_eq!(usage_of(filaments[0]), ("1.50", "2.25"));
    assert_eq!(usage_of(filaments[1]), ("0.00", "0.00"));
    assert_eq!(attr(named(&out, "metadata")[0], "value"), Some("NEW"));
}

#[test]
fn plates_ignore_root_level_filaments() {
    let xml = r#"<config><filament id="1" used_m="9" used_g="9"/>
<plate><filament id="1" used_m="1" used_g="1"/></plate></config>"#;
    let out = run(xml, &two_slots(), &identity(&[1]), "");
    let filaments = named(&out, "filament");
    assert_eq!(filaments.len(), 2);
    assert!(filaments.iter().all(|f| f.parent == "plate"));
    assert_eq!(usage_of(filaments[0]), ("1.00", "1.00"));
}

#[test]
fn many_to_one_mapping_sums_usage() {
    let xml = r#"<config><plate>
<filament id="1" used_m="1.00" used_g="2.00"/>
<filament id="2" used_m="0.50" used_g="0.25"/>
<filament id="3" used_m="3" used_g="3"/>
</plate></config>"#;
    let mapping: BTreeMap<i32, i32> = [(1, 1), (2, 1), (3, 2)].into_iter().collect();
    let out = run(xml, &two_slots(), &mapping, "");
    let filaments = named(&out, "filament");
    assert_eq!(usage_of(filaments[0]), ("1.50", "2.25"));
    assert_eq!(usage_of(filaments[1]), ("3.00", "3.00"));
}

#[test]
fn missing_usage_and_unmapped_ids_yield_zero() {
    let xml = r#"<config><plate>
<filament id="9" used_m="5" used_g="5"/>
<filament type="PLA" used_m="5" used_g="5"/>
</plate></config>"#;
    let slots = vec![
        slot("#000000", "PLA"),
        slot("#111111", "PLA"),
        slot("#222222", "PLA"),
    ];
    let out = run(xml, &slots, &identity(&[1, 2, 3]), "");
    let filaments = named(&out, "filament");
    assert_eq!(filaments.len(), 3);
    for f in &filaments {
        assert_eq!(usage_of(f), ("0.00", "0.00"));
    }
    // A plate without any filament elements also gets the full slot list.
    let out = run("<config><plate/></config>", &slots, &identity(&[1]), "");
    assert_eq!(named(&out, "filament").len(), 3);
}

#[test]
fn invalid_numeric_fields_are_skipped_like_go() {
    let xml = r#"<config><plate>
<filament id="x" used_m="7" used_g="7"/>
<filament id="" used_m="7" used_g="7"/>
<filament id="1" used_m="abc" used_g="2"/>
<filament id="1" used_m="1.5" used_g="nan"/>
<filament id="1" used_m="inf" used_g=""/>
<filament id="2" used_m="1e1" used_g=".5"/>
</plate></config>"#;
    let out = run(xml, &two_slots(), &identity(&[1, 2]), "");
    let filaments = named(&out, "filament");
    assert_eq!(usage_of(filaments[0]), ("1.50", "2.00"));
    assert_eq!(usage_of(filaments[1]), ("10.00", "0.50"));
}

#[test]
fn printer_model_metadata_is_retargeted_only_when_requested() {
    let out = run(ONE_PLATE, &two_slots(), &identity(&[1, 2]), "N2S");
    let metadata = named(&out, "metadata");
    assert_eq!(metadata.len(), 2);
    assert_eq!(attr(metadata[0], "key"), Some("index"));
    assert_eq!(attr(metadata[0], "value"), Some("1"));
    assert_eq!(attr(metadata[1], "key"), Some("printer_model_id"));
    assert_eq!(attr(metadata[1], "value"), Some("N2S"));

    let unchanged = run(ONE_PLATE, &two_slots(), &identity(&[1, 2]), "");
    assert_eq!(
        attr(named(&unchanged, "metadata")[1], "value"),
        Some("BL-P001")
    );
}

#[test]
fn unrelated_attributes_namespaces_and_comments_are_preserved() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<config xmlns:x="urn:example:x" version="2">
  <!-- keep me -->
  <plate x:flag="on">
    <x:note text="a &amp; b &lt; c" empty=""/>
    <object identify_id="7" name="part &quot;one&quot;" skipped="true"/>
    <metadata key="printer_model_id" value="OLD" extra="keep"/>
    <filament id="1" used_m="1" used_g="1"/>
  </plate>
</config>"#;
    let raw = rewrite_slice_info_xml(xml.as_bytes(), &two_slots(), &identity(&[1]), "NEW").unwrap();
    let text = String::from_utf8(raw.clone()).unwrap();
    assert!(text.contains("<!-- keep me -->"));
    assert!(text.contains("xmlns:x=\"urn:example:x\""));
    let out = parse(&raw);
    assert_eq!(attr(named(&out, "config")[0], "version"), Some("2"));
    assert_eq!(attr(named(&out, "plate")[0], "x:flag"), Some("on"));
    let note = named(&out, "note")[0];
    assert_eq!(attr(note, "text"), Some("a & b < c"));
    assert_eq!(attr(note, "empty"), Some(""));
    assert_eq!(attr(named(&out, "object")[0], "name"), Some("part \"one\""));
    let metadata = named(&out, "metadata")[0];
    assert_eq!(attr(metadata, "extra"), Some("keep"));
    assert_eq!(attr(metadata, "value"), Some("NEW"));
}

#[test]
fn slot_names_are_escaped_and_round_trip() {
    let slots = vec![slot("#12'\"<&>", "PLA <Silk> & \"Co\"")];
    let out = rewrite_slice_info_xml(
        br#"<config><plate><filament id="1" used_m="1" used_g="1"/></plate></config>"#,
        &slots,
        &identity(&[1]),
        "A&B<\"'>",
    )
    .unwrap();
    let parsed = parse(&out);
    let filament = named(&parsed, "filament")[0];
    assert_eq!(attr(filament, "color"), Some("#12'\"<&>"));
    assert_eq!(attr(filament, "type"), Some("PLA <Silk> & \"Co\""));

    let with_model = rewrite_slice_info_xml(
        br#"<config><plate><metadata key="printer_model_id" value="x"/></plate></config>"#,
        &slots,
        &BTreeMap::new(),
        "A&B<\"'>",
    )
    .unwrap();
    let parsed = parse(&with_model);
    assert_eq!(
        attr(named(&parsed, "metadata")[0], "value"),
        Some("A&B<\"'>")
    );
}

#[test]
fn malformed_xml_is_rejected() {
    let slots = two_slots();
    let mapping = identity(&[1]);
    for bad in [
        "",
        "not xml",
        "<config><plate></config>",
        "<config><plate>",
        "<config",
        "<a/><b/>",
        "<!DOCTYPE config [<!ENTITY x \"y\">]><config/>",
        "<config><filament id=\"1\" id=\"2\"/></config>",
    ] {
        assert!(
            rewrite_slice_info_xml(bad.as_bytes(), &slots, &mapping, "").is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn size_slot_and_mapping_limits_are_enforced() {
    let xml = br#"<config><plate/></config>"#;
    let mapping = identity(&[1]);

    // XML size: one byte over 8 MiB fails before parsing.
    let oversized = vec![b' '; (8 << 20) + 1];
    assert!(rewrite_slice_info_xml(&oversized, &two_slots(), &mapping, "").is_err());

    // Slot counts: 0 and 65 fail, 1 and 64 pass.
    assert!(rewrite_slice_info_xml(xml, &[], &BTreeMap::new(), "").is_err());
    let many = |n: usize| (0..n).map(|_| slot("#000000", "PLA")).collect::<Vec<_>>();
    assert!(rewrite_slice_info_xml(xml, &many(65), &mapping, "").is_err());
    assert!(rewrite_slice_info_xml(xml, &many(1), &mapping, "").is_ok());
    let out = rewrite_slice_info_xml(xml, &many(64), &mapping, "").unwrap();
    assert_eq!(named(&parse(&out), "filament").len(), 64);

    // Mapping targets must exist among the slots.
    for target in [0, -1, 3] {
        let bad: BTreeMap<i32, i32> = [(1, target)].into_iter().collect();
        assert!(rewrite_slice_info_xml(xml, &two_slots(), &bad, "").is_err());
    }

    // Characters that cannot appear in XML are rejected, not silently dropped.
    assert!(rewrite_slice_info_xml(xml, &[slot("#00\u{0}000", "PLA")], &mapping, "").is_err());
    assert!(rewrite_slice_info_xml(xml, &[slot("#000000", "PLA")], &mapping, "a\u{1}b").is_err());
}

#[test]
fn nested_plate_children_and_output_are_stable_across_runs() {
    let xml = r#"<config>
<plate><layer_filament_lists><filament_list/></layer_filament_lists>
<filament id="2" used_m="2" used_g="2"/></plate></config>"#;
    let first = rewrite_slice_info_xml(xml.as_bytes(), &two_slots(), &identity(&[2]), "M").unwrap();
    let second = rewrite_slice_info_xml(&first, &two_slots(), &identity(&[1, 2]), "M").unwrap();
    // Re-running on already rewritten output re-aggregates the new entries
    // and still produces exactly one filament per slot.
    let out = parse(&second);
    assert_eq!(named(&out, "filament").len(), 2);
    assert_eq!(named(&out, "filament_list").len(), 1);
    let out_first = parse(&first);
    assert_eq!(usage_of(named(&out_first, "filament")[1]), ("2.00", "2.00"));
}
