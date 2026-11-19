//! Experimental, standalone Rust port of Go converter.rewriteModelSettings.
//! Not wired into the CLI; nonproduction bounds apply.
//!
//! Only `<metadata key="extruder" value="N">` and
//! `<metadata key="filament_maps" value="...">` attribute values are edited.
//! Every other byte of the document is copied verbatim, so namespace
//! prefixes, attribute order, quoting and whitespace are preserved.
use std::collections::BTreeMap;

pub const MAX_INPUT_BYTES: usize = 8 << 20;
pub const MIN_SLOTS: usize = 1;
pub const MAX_SLOTS: usize = 64;
const MAX_DEPTH: usize = 256;

struct Attr<'a> {
    name: &'a str,
    value_start: usize,
    value_end: usize,
}

struct StartTag<'a> {
    name: &'a str,
    attrs: Vec<Attr<'a>>,
    self_close: bool,
    end: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn valid_xml_char(c: char) -> bool {
    matches!(c as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// Returns the end index of an XML name starting at `start` (== start if none).
fn scan_name(b: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < b.len() {
        let c = b[i];
        let ok = c.is_ascii_alphabetic()
            || c == b'_'
            || c == b':'
            || c >= 0x80
            || (i > start && (c.is_ascii_digit() || c == b'-' || c == b'.'));
        if !ok {
            break;
        }
        i += 1;
    }
    i
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    i
}

/// Decodes the five predefined entities and numeric references.
fn decode_refs(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let semi = after
            .find(';')
            .filter(|&n| n <= 16)
            .ok_or_else(|| "malformed XML: unterminated entity reference".to_owned())?;
        match &after[..semi] {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            other => {
                let num = other
                    .strip_prefix('#')
                    .ok_or_else(|| "malformed XML: unknown entity reference".to_owned())?;
                let code = match num.strip_prefix('x') {
                    Some(h) => u32::from_str_radix(h, 16),
                    None => num.parse::<u32>(),
                }
                .map_err(|_| "malformed XML: bad character reference".to_owned())?;
                let ch = char::from_u32(code)
                    .filter(|c| valid_xml_char(*c))
                    .ok_or_else(|| "malformed XML: invalid character reference".to_owned())?;
                out.push(ch);
            }
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#xA;"),
            '\t' => out.push_str("&#x9;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
    out
}

fn parse_start(text: &str, start: usize) -> Result<StartTag<'_>, String> {
    let b = text.as_bytes();
    let eof = || "malformed XML: unexpected end of input in tag".to_owned();
    let mut i = start + 1;
    let name_end = scan_name(b, i);
    if name_end == i {
        return Err("malformed XML: invalid element name".to_owned());
    }
    let name = &text[i..name_end];
    i = name_end;
    let mut attrs: Vec<Attr> = Vec::new();
    loop {
        let after_ws = skip_ws(b, i);
        let had_ws = after_ws > i;
        i = after_ws;
        let c = *b.get(i).ok_or_else(eof)?;
        if c == b'>' {
            return Ok(StartTag {
                name,
                attrs,
                self_close: false,
                end: i + 1,
            });
        }
        if c == b'/' {
            if b.get(i + 1) == Some(&b'>') {
                return Ok(StartTag {
                    name,
                    attrs,
                    self_close: true,
                    end: i + 2,
                });
            }
            return Err("malformed XML: stray '/' in tag".to_owned());
        }
        if !had_ws {
            return Err("malformed XML: missing whitespace between attributes".to_owned());
        }
        let an = i;
        i = scan_name(b, i);
        if i == an {
            return Err("malformed XML: invalid attribute name".to_owned());
        }
        let aname = &text[an..i];
        i = skip_ws(b, i);
        if b.get(i) != Some(&b'=') {
            return Err("malformed XML: attribute without '='".to_owned());
        }
        i = skip_ws(b, i + 1);
        let q = *b.get(i).ok_or_else(eof)?;
        if q != b'"' && q != b'\'' {
            return Err("malformed XML: unquoted attribute value".to_owned());
        }
        i += 1;
        let vs = i;
        while i < b.len() && b[i] != q {
            if b[i] == b'<' {
                return Err("malformed XML: '<' in attribute value".to_owned());
            }
            i += 1;
        }
        if i >= b.len() {
            return Err(eof());
        }
        let ve = i;
        i += 1;
        decode_refs(&text[vs..ve])?;
        if attrs.iter().any(|a| a.name == aname) {
            return Err("malformed XML: duplicate attribute".to_owned());
        }
        attrs.push(Attr {
            name: aname,
            value_start: vs,
            value_end: ve,
        });
    }
}

/// Computes replacement text for a `value` attribute of a metadata element.
fn new_value(key: &str, old: &str, mapping: &BTreeMap<i32, i32>, slots: usize) -> Option<String> {
    match key {
        "extruder" => {
            let v: i64 = old.trim().parse().ok()?;
            if let Some(dst) = i32::try_from(v).ok().and_then(|k| mapping.get(&k)) {
                Some(dst.to_string())
            } else if v > slots as i64 {
                Some("1".to_owned())
            } else {
                None
            }
        }
        "filament_maps" => Some(vec!["1"; slots].join(" ")),
        _ => None,
    }
}

/// Rewrites Bambu `Metadata/model_settings.config` XML. See module docs.
pub fn rewrite_model_settings(
    raw: &[u8],
    mapping: &BTreeMap<i32, i32>,
    slots: usize,
) -> Result<Vec<u8>, String> {
    if raw.len() > MAX_INPUT_BYTES {
        return Err("model settings exceed 8 MiB limit".to_owned());
    }
    if !(MIN_SLOTS..=MAX_SLOTS).contains(&slots) {
        return Err("slots must be within 1..=64".to_owned());
    }
    let text =
        std::str::from_utf8(raw).map_err(|_| "model settings are not valid UTF-8".to_owned())?;
    let b = text.as_bytes();
    if b.iter().any(|&c| c < 0x20 && !is_ws(c)) {
        return Err("malformed XML: control character".to_owned());
    }

    let mut i = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let mut last = 0usize;
    let mut out: Vec<u8> = Vec::with_capacity(raw.len() + 64);
    let mut stack: Vec<&str> = Vec::new();
    let mut root_done = false;

    while i < b.len() {
        if b[i] != b'<' {
            let end = text[i..].find('<').map_or(b.len(), |n| i + n);
            let seg = &text[i..end];
            if stack.is_empty() {
                if !seg.bytes().all(is_ws) {
                    return Err("malformed XML: text outside root element".to_owned());
                }
            } else {
                if seg.contains("]]>") {
                    return Err("malformed XML: ']]>' in text".to_owned());
                }
                decode_refs(seg)?;
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if let Some(after_prefix) = rest.strip_prefix("<!--") {
            let e = after_prefix
                .find("-->")
                .ok_or_else(|| "malformed XML: unterminated comment".to_owned())?;
            let body = &after_prefix[..e];
            if body.contains("--") || body.ends_with('-') {
                return Err("malformed XML: '--' in comment".to_owned());
            }
            i += 4 + e + 3;
        } else if let Some(after_prefix) = rest.strip_prefix("<![CDATA[") {
            if stack.is_empty() {
                return Err("malformed XML: CDATA outside root element".to_owned());
            }
            let e = after_prefix
                .find("]]>")
                .ok_or_else(|| "malformed XML: unterminated CDATA".to_owned())?;
            i += 9 + e + 3;
        } else if let Some(after_prefix) = rest.strip_prefix("<?") {
            if scan_name(b, i + 2) == i + 2 {
                return Err("malformed XML: invalid processing instruction".to_owned());
            }
            let e = after_prefix
                .find("?>")
                .ok_or_else(|| "malformed XML: unterminated processing instruction".to_owned())?;
            i += 2 + e + 2;
        } else if rest.starts_with("<!") {
            return Err("DOCTYPE and XML directives are not supported".to_owned());
        } else if rest.starts_with("</") {
            let ns = i + 2;
            let ne = scan_name(b, ns);
            if ne == ns {
                return Err("malformed XML: invalid end tag".to_owned());
            }
            let gt = skip_ws(b, ne);
            if b.get(gt) != Some(&b'>') {
                return Err("malformed XML: unterminated end tag".to_owned());
            }
            match stack.pop() {
                Some(open) if open == &text[ns..ne] => {}
                _ => return Err("malformed XML: mismatched end tag".to_owned()),
            }
            if stack.is_empty() {
                root_done = true;
            }
            i = gt + 1;
        } else {
            if stack.is_empty() && root_done {
                return Err("malformed XML: multiple root elements".to_owned());
            }
            let tag = parse_start(text, i)?;
            if local(tag.name) == "metadata" {
                let key = match tag.attrs.iter().find(|a| local(a.name) == "key") {
                    Some(a) => decode_refs(&text[a.value_start..a.value_end])?,
                    None => String::new(),
                };
                for a in tag.attrs.iter().filter(|a| local(a.name) == "value") {
                    let old = decode_refs(&text[a.value_start..a.value_end])?;
                    if let Some(new) = new_value(&key, &old, mapping, slots) {
                        if new != old {
                            out.extend_from_slice(&b[last..a.value_start]);
                            out.extend_from_slice(escape_attr(&new).as_bytes());
                            last = a.value_end;
                        }
                    }
                }
            }
            if tag.self_close {
                if stack.is_empty() {
                    root_done = true;
                }
            } else {
                if stack.len() >= MAX_DEPTH {
                    return Err("malformed XML: nesting too deep".to_owned());
                }
                stack.push(tag.name);
            }
            i = tag.end;
        }
    }
    if !stack.is_empty() {
        return Err("malformed XML: unclosed element".to_owned());
    }
    if !root_done {
        return Err("malformed XML: no root element".to_owned());
    }
    out.extend_from_slice(&b[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_attr_escapes_markup_and_whitespace() {
        assert_eq!(
            escape_attr("a&b<c>\"'\n\t\r"),
            "a&amp;b&lt;c&gt;&quot;&apos;&#xA;&#x9;&#xD;"
        );
    }

    #[test]
    fn decode_refs_handles_numeric_and_rejects_unknown() {
        assert_eq!(decode_refs("&#65;&#x42;&amp;").unwrap(), "AB&");
        assert!(decode_refs("&bogus;").is_err());
        assert!(decode_refs("&amp").is_err());
    }
}
