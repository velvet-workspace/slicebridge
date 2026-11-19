#[path = "../src/modelsettings.rs"]
mod modelsettings;

use modelsettings::{rewrite_model_settings, MAX_INPUT_BYTES};
use std::collections::BTreeMap;

fn map(pairs: &[(i32, i32)]) -> BTreeMap<i32, i32> {
    pairs.iter().copied().collect()
}

fn run(xml: &str, mapping: &BTreeMap<i32, i32>, slots: usize) -> String {
    let out = rewrite_model_settings(xml.as_bytes(), mapping, slots).expect("rewrite succeeds");
    String::from_utf8(out).expect("output is UTF-8")
}

#[test]
fn mapped_extruder_is_rewritten_and_mapping_wins_over_overflow() {
    let xml = r#"<config><object id="1"><metadata key="extruder" value="2"/><metadata key="extruder" value="9"/><metadata key="name" value="2"/></object></config>"#;
    let out = run(xml, &map(&[(2, 4), (9, 3)]), 4);
    assert_eq!(
        out,
        r#"<config><object id="1"><metadata key="extruder" value="4"/><metadata key="extruder" value="3"/><metadata key="name" value="2"/></object></config>"#
    );
}

#[test]
fn mapped_value_tolerates_whitespace_and_single_quotes() {
    let xml = "<c><metadata key='extruder' value=' 2 '/></c>";
    assert_eq!(
        run(xml, &map(&[(2, 5)]), 8),
        "<c><metadata key='extruder' value='5'/></c>"
    );
}

#[test]
fn overflow_resets_to_one_but_in_range_unmapped_is_kept() {
    let xml = r#"<c><metadata key="extruder" value="9"/><metadata key="extruder" value="3"/><metadata key="extruder" value="-2"/><metadata key="extruder" value="99999999999"/></c>"#;
    let out = run(xml, &BTreeMap::new(), 4);
    assert_eq!(
        out,
        r#"<c><metadata key="extruder" value="1"/><metadata key="extruder" value="3"/><metadata key="extruder" value="-2"/><metadata key="extruder" value="1"/></c>"#
    );
}

#[test]
fn malformed_integers_are_left_untouched() {
    let xml = r#"<c><metadata key="extruder" value="abc"/><metadata key="extruder" value=""/><metadata key="extruder" value="2.5"/><metadata key="extruder" value="99999999999999999999999"/></c>"#;
    assert_eq!(run(xml, &map(&[(2, 3)]), 4), xml);
}

#[test]
fn missing_keys_and_values_leave_bytes_identical() {
    let xml = "<?xml version=\"1.0\"?>\n<config>\n  <metadata value=\"9\"/>\n  <metadata key=\"other\" value=\"9\"/>\n  <metadata key=\"extruder\"/>\n  <plate/>\n</config>\n";
    assert_eq!(run(xml, &map(&[(9, 2)]), 4), xml);
}

#[test]
fn filament_maps_become_slot_count_of_ones() {
    let xml = r#"<c><metadata key="filament_maps" value="1 2 2 1"/></c>"#;
    assert_eq!(
        run(xml, &BTreeMap::new(), 3),
        r#"<c><metadata key="filament_maps" value="1 1 1"/></c>"#
    );
    assert_eq!(
        run(xml, &BTreeMap::new(), 1),
        r#"<c><metadata key="filament_maps" value="1"/></c>"#
    );
    assert_eq!(run(xml, &BTreeMap::new(), 64).matches('1').count(), 64);
}

#[test]
fn namespace_prefixes_are_matched_by_local_name_and_preserved() {
    let xml = r#"<b:config xmlns:b="urn:bambu"><b:metadata b:key="extruder" b:value="9"/><x:metadata key="filament_maps" x:value="2 2"/></b:config>"#;
    let out = run(xml, &BTreeMap::new(), 2);
    assert_eq!(
        out,
        r#"<b:config xmlns:b="urn:bambu"><b:metadata b:key="extruder" b:value="1"/><x:metadata key="filament_maps" x:value="1 1"/></b:config>"#
    );
}

#[test]
fn nonmetadata_elements_and_other_attributes_are_not_changed() {
    let xml = r#"<config><part key="extruder" value="9"/><metadata key="extruder" other="9" value="9" z="&amp;"/></config>"#;
    let out = run(xml, &BTreeMap::new(), 4);
    assert_eq!(
        out,
        r#"<config><part key="extruder" value="9"/><metadata key="extruder" other="9" value="1" z="&amp;"/></config>"#
    );
}

#[test]
fn comments_cdata_pi_entities_and_whitespace_are_preserved() {
    let xml = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- note -->\n<config a=\"x &lt; y &#65;\">\n  <t>a &amp; b &#x42; <![CDATA[<raw & ]]></t>\n  <metadata key=\"extruder\" value=\"7\"></metadata>\n  <m  k = 'v' />\n</config>\n";
    let expected = xml.replace("value=\"7\"", "value=\"1\"");
    assert_eq!(run(xml, &BTreeMap::new(), 2), expected);
}

#[test]
fn encoded_key_is_decoded_before_matching() {
    let xml = r#"<c><metadata key="extr&#117;der" value="9"/></c>"#;
    assert_eq!(
        run(xml, &BTreeMap::new(), 2),
        r#"<c><metadata key="extr&#117;der" value="1"/></c>"#
    );
}

#[test]
fn output_is_well_formed_and_idempotent() {
    let xml = r#"<config><metadata key="extruder" value="9"/><metadata key="filament_maps" value="3 4"/><metadata key="name" value="a &amp; &lt;b&gt; &quot;q&quot;"/></config>"#;
    let once = rewrite_model_settings(xml.as_bytes(), &map(&[(5, 2)]), 2).unwrap();
    // The output must itself parse and be a fixed point of the rewrite.
    let twice = rewrite_model_settings(&once, &map(&[(5, 2)]), 2).unwrap();
    assert_eq!(once, twice);
    let text = String::from_utf8(once).unwrap();
    assert!(text.contains(r#"value="a &amp; &lt;b&gt; &quot;q&quot;""#));
    assert!(text.contains(r#"key="filament_maps" value="1 1""#));
}

#[test]
fn malformed_xml_is_rejected() {
    let cases: &[&str] = &[
        "",
        "   ",
        "<a>",
        "<a></b>",
        "<a><b></a></b>",
        "</a>",
        "<a/><b/>",
        "text<a/>",
        "<a/>text",
        "<a b=c/>",
        "<a b=\"1\" b=\"2\"/>",
        "<a b=\"1\"c=\"2\"/>",
        "<a b=\"<\"/>",
        "<a b=\"1/>",
        "<a>&bogus;</a>",
        "<a>&amp</a>",
        "<a b=\"&#xZZ;\"/>",
        "<a>]]></a>",
        "<a><!-- x -- y --></a>",
        "<a><!-- open</a>",
        "<a><![CDATA[ open</a>",
        "<a><?pi open</a>",
        "<!DOCTYPE a [<!ENTITY e \"x\">]><a/>",
        "<a\u{1}/>",
        "<1a/>",
        "<a / >",
    ];
    for case in cases {
        let r = rewrite_model_settings(case.as_bytes(), &BTreeMap::new(), 4);
        assert!(r.is_err(), "expected error for {case:?}, got {r:?}");
    }
    assert!(
        rewrite_model_settings(&[0xff, 0xfe, b'<', b'a', b'/', b'>'], &BTreeMap::new(), 4).is_err()
    );
}

#[test]
fn slots_and_input_size_bounds_are_enforced() {
    let xml = b"<a/>";
    for bad in [0usize, 65, 1000] {
        assert!(
            rewrite_model_settings(xml, &BTreeMap::new(), bad).is_err(),
            "slots {bad}"
        );
    }
    for ok in [1usize, 64] {
        assert!(
            rewrite_model_settings(xml, &BTreeMap::new(), ok).is_ok(),
            "slots {ok}"
        );
    }

    let mut at_limit = b"<a/>".to_vec();
    at_limit.resize(MAX_INPUT_BYTES, b' ');
    assert!(rewrite_model_settings(&at_limit, &BTreeMap::new(), 4).is_ok());
    at_limit.push(b' ');
    let err = rewrite_model_settings(&at_limit, &BTreeMap::new(), 4).unwrap_err();
    assert!(err.contains("8 MiB"), "{err}");
}

#[test]
fn excessive_nesting_is_rejected() {
    let depth = 1000;
    let xml = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    assert!(rewrite_model_settings(xml.as_bytes(), &BTreeMap::new(), 4).is_err());
}
