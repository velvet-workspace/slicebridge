//! Bounded reader for PrusaSlicer INI profiles, ported from
//! `native/presets/prusa_ini.go`. Only `[type:name]` headers and `key = value`
//! lines are accepted. Values stay raw text: nothing read here is evaluated,
//! expanded, or used to open another file.

use std::collections::BTreeMap;

pub(crate) const MAX_PRUSA_INI_BYTES: usize = 8 << 20;
pub(crate) const MAX_PRUSA_INI_LINE_BYTES: usize = 64 << 10;
pub(crate) const MAX_PRUSA_INI_SECTIONS: usize = 1 << 15;
pub(crate) const MAX_PRUSA_INI_KEYS: usize = 1 << 18;
pub(crate) const MAX_PRUSA_SECTION_NAME_BYTES: usize = 256;
pub(crate) const MAX_PRUSA_VALUE_BYTES: usize = 16 << 10;
pub(crate) const MAX_PRUSA_LIST_ITEMS: usize = 256;

/// Raw, trimmed values of one section, keyed by setting name.
pub(crate) type PrusaSection = BTreeMap<String, String>;

/// Parsed INI document. Sections keep file order so discovery is deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrusaIni {
    order: Vec<String>,
    sections: BTreeMap<String, PrusaSection>,
}

impl PrusaIni {
    pub(crate) fn section_names(&self) -> &[String] {
        &self.order
    }

    pub(crate) fn section(&self, name: &str) -> Option<&PrusaSection> {
        self.sections.get(name)
    }
}

pub(crate) fn parse_prusa_ini(data: &[u8]) -> Result<PrusaIni, String> {
    if data.len() > MAX_PRUSA_INI_BYTES {
        return Err(format!("file exceeds {MAX_PRUSA_INI_BYTES} bytes"));
    }
    if data.contains(&0) {
        return Err("file contains NUL bytes".into());
    }
    // Go keeps arbitrary bytes; this port fails closed on anything that is not UTF-8.
    let content = std::str::from_utf8(data).map_err(|_| "file is not valid UTF-8".to_owned())?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let mut ini = PrusaIni::default();
    // The section being filled. It joins `ini.sections` when the next header or EOF arrives.
    let mut current: Option<(String, PrusaSection)> = None;
    let mut keys = 0;
    for (index, raw_line) in content.split('\n').enumerate() {
        let number = index + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.len() > MAX_PRUSA_INI_LINE_BYTES {
            return Err(format!(
                "line {number} exceeds {MAX_PRUSA_INI_LINE_BYTES} bytes"
            ));
        }
        if has_prusa_control(line) {
            return Err(format!("line {number} contains control characters"));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(header) = trimmed.strip_prefix('[') {
            let name = header
                .strip_suffix(']')
                .ok_or_else(|| format!("line {number} has an unterminated section header"))?
                .trim();
            if name.is_empty()
                || name.len() > MAX_PRUSA_SECTION_NAME_BYTES
                || name.contains(['[', ']'])
            {
                return Err(format!("line {number} has an invalid section name"));
            }
            if let Some((previous, section)) = current.take() {
                ini.sections.insert(previous, section);
            }
            if ini.sections.contains_key(name) {
                return Err(format!("section {name:?} is defined twice"));
            }
            if ini.order.len() >= MAX_PRUSA_INI_SECTIONS {
                return Err(format!("more than {MAX_PRUSA_INI_SECTIONS} sections"));
            }
            ini.order.push(name.to_owned());
            current = Some((name.to_owned(), PrusaSection::new()));
            continue;
        }
        let Some((_, section)) = current.as_mut() else {
            return Err(format!("line {number} sits outside any section"));
        };
        let Some(separator) = trimmed.find('=') else {
            return Err(format!("line {number} is not a key = value setting"));
        };
        let key = trimmed[..separator].trim();
        let value = trimmed[separator + 1..].trim();
        if !is_valid_prusa_key(key) {
            return Err(format!("line {number} has an invalid key"));
        }
        if section.contains_key(key) {
            return Err(format!("line {number} repeats key {key:?}"));
        }
        if value.len() > MAX_PRUSA_VALUE_BYTES {
            return Err(format!(
                "line {number} value exceeds {MAX_PRUSA_VALUE_BYTES} bytes"
            ));
        }
        keys += 1;
        if keys > MAX_PRUSA_INI_KEYS {
            return Err(format!("more than {MAX_PRUSA_INI_KEYS} settings"));
        }
        section.insert(key.to_owned(), value.to_owned());
    }
    if let Some((name, section)) = current {
        ini.sections.insert(name, section);
    }
    Ok(ini)
}

/// Reports control characters other than tab. CR is stripped before this check,
/// so Windows line endings are accepted.
fn has_prusa_control(line: &str) -> bool {
    line.bytes()
        .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
}

/// Matches Go's `^[a-z0-9_]{1,128}(\[[0-9]{1,3}\])?$`: a name with an optional
/// list index such as `dynamic_overhang_speeds[0]`.
fn is_valid_prusa_key(key: &str) -> bool {
    let (name, index) = match key.find('[') {
        Some(open) => (&key[..open], Some(&key[open..])),
        None => (key, None),
    };
    let name_ok = (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    let index_ok = match index {
        None => true,
        Some(index) => index
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .is_some_and(|digits| {
                (1..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
            }),
    };
    name_ok && index_ok
}

/// Splits a per-tool value. PrusaSlicer writes numeric vectors such as
/// `nozzle_diameter` with commas, and string lists with semicolons and quotes.
/// Values containing `;` or `"` use [`split_prusa_list`]. Anything else splits on
/// commas and drops empty items.
pub(crate) fn split_prusa_values(raw: &str) -> Result<Vec<String>, String> {
    if raw.contains([';', '"']) {
        return split_prusa_list(raw);
    }
    let items: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    if items.len() > MAX_PRUSA_LIST_ITEMS {
        return Err(format!("list has more than {MAX_PRUSA_LIST_ITEMS} items"));
    }
    Ok(items)
}

/// Splits a PrusaSlicer list value such as `"A;B";C`. Separators inside double
/// quotes are kept, and a backslash escapes a quote or backslash inside quotes.
/// Empty items are dropped, so an empty value means an empty list.
pub(crate) fn split_prusa_list(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut items: Vec<String> = Vec::new();
    let mut item = String::new();
    let mut quoted = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted && c == '\\' {
            if let Some(escaped) = chars.next_if(|next| matches!(*next, '"' | '\\')) {
                item.push(escaped);
                continue;
            }
        }
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                items.push(item.trim().to_owned());
                item.clear();
                // Like the Go source, the bound is checked when a separator
                // closes an item. The final item is appended unchecked below.
                if items.len() > MAX_PRUSA_LIST_ITEMS {
                    return Err(format!("list has more than {MAX_PRUSA_LIST_ITEMS} items"));
                }
            }
            _ => item.push(c),
        }
    }
    if quoted {
        return Err("list has an unterminated quote".into());
    }
    items.push(item.trim().to_owned());
    items.retain(|value| !value.is_empty());
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value<'a>(ini: &'a PrusaIni, section: &str, key: &str) -> Option<&'a str> {
        ini.section(section)?.get(key).map(String::as_str)
    }

    #[test]
    fn accepts_real_profile_shapes() {
        let data = b"\xef\xbb\xbf# comment\r\n[vendor]\r\nname = QIDITechnology\r\n\r\n\
            [print:*X-Plus 4*]\r\ninherits = *common*; *0.08mm*\r\nlayer_height = 0.2\r\n";
        let ini = parse_prusa_ini(data).expect("valid profile rejected");
        let names: Vec<&str> = ini.section_names().iter().map(String::as_str).collect();
        assert_eq!(names, ["vendor", "print:*X-Plus 4*"]);
        assert_eq!(value(&ini, "vendor", "name"), Some("QIDITechnology"));
        assert_eq!(
            value(&ini, "print:*X-Plus 4*", "inherits"),
            Some("*common*; *0.08mm*")
        );
    }

    #[test]
    fn rejects_malformed_input() {
        let long_line = "a".repeat(MAX_PRUSA_INI_LINE_BYTES + 1);
        let long_value = format!(
            "[printer:a]\nx = {}\n",
            "v".repeat(MAX_PRUSA_VALUE_BYTES + 1)
        );
        let cases: Vec<(&str, String)> = vec![
            ("key outside section", "name = x\n".into()),
            ("no equals sign", "[printer:a]\nnozzle 0.4\n".into()),
            ("unterminated header", "[printer:a\n".into()),
            ("empty section name", "[]\n".into()),
            ("duplicate section", "[printer:a]\n[printer:a]\n".into()),
            ("duplicate key", "[printer:a]\nx = 1\nx = 2\n".into()),
            ("invalid key chars", "[printer:a]\nBad-Key = 1\n".into()),
            ("NUL byte", "[printer:a]\x00\n".into()),
            ("control character", "[printer:a]\nx = a\x01b\n".into()),
            ("oversized line", format!("[printer:a]\n{long_line}\n")),
            ("nested bracket name", "[printer:[a]]\n".into()),
            ("oversized value", long_value),
        ];
        for (name, data) in cases {
            assert!(
                parse_prusa_ini(data.as_bytes()).is_err(),
                "{name}: malformed profile accepted"
            );
        }
        assert!(parse_prusa_ini(b"[printer:a]\nx = \xff\n").is_err());
        assert!(parse_prusa_ini(&vec![0u8; MAX_PRUSA_INI_BYTES + 1]).is_err());
        assert!(parse_prusa_ini(&vec![b'#'; MAX_PRUSA_INI_BYTES + 1]).is_err());
    }

    #[test]
    fn strips_only_one_byte_order_mark() {
        assert!(parse_prusa_ini("\u{feff}\u{feff}[printer:a]\n".as_bytes()).is_err());
        let ini = parse_prusa_ini("\u{feff}[printer:a]\n".as_bytes()).unwrap();
        assert_eq!(ini.section_names(), ["printer:a"]);
    }

    #[test]
    fn bounds_section_count() {
        let many: String = (0..=MAX_PRUSA_INI_SECTIONS)
            .map(|i| format!("[print:p{i}]\n"))
            .collect();
        assert!(parse_prusa_ini(many.as_bytes()).is_err());
        let at_limit: String = (0..MAX_PRUSA_INI_SECTIONS)
            .map(|i| format!("[print:p{i}]\n"))
            .collect();
        assert_eq!(
            parse_prusa_ini(at_limit.as_bytes())
                .unwrap()
                .section_names()
                .len(),
            MAX_PRUSA_INI_SECTIONS
        );
    }

    #[test]
    fn validates_key_syntax() {
        assert!(is_valid_prusa_key("layer_height"));
        assert!(is_valid_prusa_key("dynamic_overhang_speeds[0]"));
        assert!(is_valid_prusa_key("nozzle_diameter[999]"));
        assert!(is_valid_prusa_key(&"k".repeat(128)));
        let too_long = "k".repeat(129);
        for key in [
            "",
            "A",
            "a-b",
            "[0]",
            "a[]",
            "a[1000]",
            "a[1]x",
            "a[1][2]",
            "a[",
            too_long.as_str(),
        ] {
            assert!(!is_valid_prusa_key(key), "{key:?} accepted");
        }
    }

    #[test]
    fn splits_prusa_list() {
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("", vec![]),
            ("0.4", vec!["0.4"]),
            ("*common*; *0.08mm*", vec!["*common*", "*0.08mm*"]),
            (
                r#""Printer A";"Printer; B""#,
                vec!["Printer A", "Printer; B"],
            ),
            (r#""say \"hi\"""#, vec![r#"say "hi""#]),
            (r#""back\\slash""#, vec![r"back\slash"]),
            (";;", vec![]),
        ];
        for (raw, want) in cases {
            assert_eq!(split_prusa_list(raw).unwrap(), want, "split {raw:?}");
        }
        assert!(split_prusa_list(r#""unterminated"#).is_err());
    }

    #[test]
    fn splits_prusa_values_by_separator_kind() {
        assert_eq!(split_prusa_values("0.4, 0.6,").unwrap(), ["0.4", "0.6"]);
        assert_eq!(split_prusa_values(" , ").unwrap(), Vec::<String>::new());
        assert_eq!(split_prusa_values("0.4;0.6").unwrap(), ["0.4", "0.6"]);
        assert_eq!(split_prusa_values(r#""a,b";c"#).unwrap(), ["a,b", "c"]);
        let too_many = vec!["1"; MAX_PRUSA_LIST_ITEMS + 1].join(",");
        assert!(split_prusa_values(&too_many).is_err());
    }
}
