//! Compatibility-condition evaluator, ported from
//! `native/presets/prusa_condition.go`. Supports `==`, `!=`, `=~`, `!~`, `and`,
//! `or`, parentheses, double-quoted strings, numbers, and setting names with an
//! optional list index such as `name[0]`. Regular expressions use the `/pattern/`
//! form and the Rust `regex` crate, which runs in linear time. Anything else is an
//! error. A preset whose condition errors is treated as incompatible, never guessed.

use crate::prusa_ini::{split_prusa_values, PrusaSection};

pub(crate) const MAX_PRUSA_CONDITION_BYTES: usize = 2048;
pub(crate) const MAX_PRUSA_CONDITION_TOKENS: usize = 256;
pub(crate) const MAX_PRUSA_REGEX_BYTES: usize = 256;
/// Rust-side bound on the compiled regex size. The Go source relies on RE2 limits.
const MAX_PRUSA_REGEX_COMPILED_BYTES: usize = 1 << 20;
const MAX_PRUSA_CONDITION_INDEX: i64 = 63;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    Equal,
    NotEqual,
    Matches,
    NotMatches,
    And,
    Or,
    /// A double-quoted string or a number, compared as raw text.
    Literal(String),
    Regex(String),
    Setting {
        name: String,
        index: Option<usize>,
    },
    End,
}

/// Evaluates a compatibility condition against one section's raw values. Missing
/// settings read as empty. Both sides of `and`/`or` are always parsed, so a syntax
/// error cannot hide behind a short-circuit.
pub(crate) fn evaluate_prusa_condition(expr: &str, values: &PrusaSection) -> Result<bool, String> {
    let tokens = tokenize_prusa_condition(expr.trim())?;
    let mut parser = ConditionParser {
        tokens,
        pos: 0,
        values,
    };
    let result = parser.or_expr()?;
    if !matches!(parser.peek(), Token::End) {
        return Err("unexpected token in compatibility condition".into());
    }
    Ok(result)
}

fn tokenize_prusa_condition(expr: &str) -> Result<Vec<Token>, String> {
    if expr.is_empty() {
        return Err("compatibility condition is empty".into());
    }
    if expr.len() > MAX_PRUSA_CONDITION_BYTES {
        return Err(format!(
            "compatibility condition exceeds {MAX_PRUSA_CONDITION_BYTES} bytes"
        ));
    }
    let bytes = expr.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        let (token, next) = match c {
            b'(' => (Token::LParen, i + 1),
            b')' => (Token::RParen, i + 1),
            b'=' | b'!' if matches!(bytes.get(i + 1), Some(b'=' | b'~')) => {
                let token = match (c, bytes[i + 1]) {
                    (b'=', b'=') => Token::Equal,
                    (b'!', b'=') => Token::NotEqual,
                    (b'=', _) => Token::Matches,
                    _ => Token::NotMatches,
                };
                (token, i + 2)
            }
            b'/' => scan_regex(expr, i)?,
            b'"' => scan_string(expr, i)?,
            b'0'..=b'9' => {
                let end = scan_while(bytes, i, |b| b.is_ascii_digit() || b == b'.');
                (Token::Literal(expr[i..end].to_owned()), end)
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => scan_word(expr, i)?,
            _ => {
                return Err(
                    "compatibility condition uses an unsupported operator or character".into(),
                )
            }
        };
        tokens.push(token);
        if tokens.len() > MAX_PRUSA_CONDITION_TOKENS {
            return Err(format!(
                "compatibility condition has more than {MAX_PRUSA_CONDITION_TOKENS} tokens"
            ));
        }
        i = next;
    }
    tokens.push(Token::End);
    Ok(tokens)
}

fn scan_while(bytes: &[u8], start: usize, keep: impl Fn(u8) -> bool) -> usize {
    let mut end = start;
    while end < bytes.len() && keep(bytes[end]) {
        end += 1;
    }
    end
}

/// Scans `/pattern/`. A backslash skips the following byte, so `\/` does not close
/// the pattern. The returned text keeps the escapes for the regex engine.
fn scan_regex(expr: &str, start: usize) -> Result<(Token, usize), String> {
    let bytes = expr.as_bytes();
    let mut end = start + 1;
    let mut closed = false;
    while end < bytes.len() && !closed {
        match bytes[end] {
            b'\\' => end += 1,
            b'/' => closed = true,
            _ => {}
        }
        if !closed {
            end += 1;
        }
    }
    if !closed || end - start - 1 > MAX_PRUSA_REGEX_BYTES {
        return Err(
            "compatibility condition has an unterminated or oversized regular expression".into(),
        );
    }
    Ok((Token::Regex(expr[start + 1..end].to_owned()), end + 1))
}

/// Scans `"text"`. Strings have no escape sequences.
fn scan_string(expr: &str, start: usize) -> Result<(Token, usize), String> {
    let Some(len) = expr[start + 1..].find('"') else {
        return Err("compatibility condition has an unterminated string".into());
    };
    let end = start + 1 + len;
    Ok((Token::Literal(expr[start + 1..end].to_owned()), end + 1))
}

/// Scans a bare word: `and`, `or`, or a setting name with an optional `[index]`.
fn scan_word(expr: &str, start: usize) -> Result<(Token, usize), String> {
    let bytes = expr.as_bytes();
    let end = scan_while(bytes, start, |b| b.is_ascii_alphanumeric() || b == b'_');
    let word = &expr[start..end];
    match word {
        "and" => Ok((Token::And, end)),
        "or" => Ok((Token::Or, end)),
        _ => {
            if bytes.get(end) != Some(&b'[') {
                let token = Token::Setting {
                    name: word.to_owned(),
                    index: None,
                };
                return Ok((token, end));
            }
            let close = end
                + expr[end..]
                    .find(']')
                    .ok_or("compatibility condition has an unterminated index")?;
            // Like Go's strconv.Atoi, this accepts a leading sign such as "+1".
            let index = expr[end + 1..close]
                .parse::<i64>()
                .ok()
                .filter(|index| (0..=MAX_PRUSA_CONDITION_INDEX).contains(index))
                .ok_or("compatibility condition has an unsupported index")?;
            let token = Token::Setting {
                name: word.to_owned(),
                index: Some(index as usize),
            };
            Ok((token, close + 1))
        }
    }
}

struct ConditionParser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    values: &'a PrusaSection,
}

impl ConditionParser<'_> {
    // `tokens` always ends with `Token::End`, and `advance` never moves past it,
    // so `pos` stays in bounds for any input.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if token != Token::End {
            self.pos += 1;
        }
        token
    }

    fn or_expr(&mut self) -> Result<bool, String> {
        let mut left = self.and_expr()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.and_expr()?;
            left = left || right;
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<bool, String> {
        let mut left = self.primary()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.primary()?;
            left = left && right;
        }
        Ok(left)
    }

    fn primary(&mut self) -> Result<bool, String> {
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            let value = self.or_expr()?;
            if !matches!(self.advance(), Token::RParen) {
                return Err("compatibility condition is missing a closing parenthesis".into());
            }
            return Ok(value);
        }
        let left = self.operand()?;
        let operator = self.advance();
        match operator {
            Token::Matches | Token::NotMatches => {
                let Token::Regex(pattern) = self.advance() else {
                    return Err("regular expression comparison needs a /pattern/".into());
                };
                let matched = compile_prusa_regex(&pattern)?.is_match(&left);
                Ok(if operator == Token::NotMatches {
                    !matched
                } else {
                    matched
                })
            }
            Token::Equal | Token::NotEqual => {
                let right = self.operand()?;
                let equal = prusa_values_equal(&left, &right);
                Ok(if operator == Token::NotEqual {
                    !equal
                } else {
                    equal
                })
            }
            _ => Err("compatibility condition needs a comparison".into()),
        }
    }

    /// Returns a literal's text or a setting's value. An absent setting reads as
    /// empty. An indexed setting reads the item at that index of its list value.
    fn operand(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Literal(text) => Ok(text),
            Token::Setting { name, index } => {
                let raw = self.values.get(&name).map_or("", String::as_str);
                let Some(index) = index else {
                    return Ok(raw.to_owned());
                };
                let items =
                    split_prusa_values(raw).map_err(|_| format!("setting {name} is malformed"))?;
                Ok(items.get(index).cloned().unwrap_or_default())
            }
            _ => Err("compatibility condition has a missing operand".into()),
        }
    }
}

// Go regexp/RE2 treats Perl \d, \w, \s and word boundaries as ASCII. The
// Rust regex crate defaults to Unicode for those shorthands. Translate just
// their lexical forms while keeping Unicode \p classes and literal UTF-8 text.
// Never enable backtracking or silently accept a Rust-only escape/flag.
fn go_regex_pattern(pattern: &str) -> Result<String, String> {
    let invalid = || "compatibility condition has an invalid regular expression".to_owned();
    let mut translated = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    let mut at_class_start = false;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let next = chars.next().ok_or_else(invalid)?;
            if next == 'u' {
                // Go's RE2 has \x{....}, not Rust's \u....
                return Err(invalid());
            }
            if next == 'b' && !in_class && chars.peek() == Some(&'{') {
                // Go treats \b{start} as an ASCII word boundary followed by
                // literal braces, not Rust's named-boundary syntax.
                let look: String = chars.clone().take(32).collect();
                if let Some(end) = look.find('}') {
                    let body = &look[1..end];
                    if body.starts_with(|c: char| c.is_ascii_alphabetic()) {
                        translated.push_str(r"(?-u:\b)\{");
                        translated.push_str(&regex::escape(body));
                        translated.push_str(r"\}");
                        for _ in 0..=end {
                            chars.next();
                        }
                        continue;
                    }
                }
            }
            if next == 'Q' {
                // Go supports \Q...\E literal quoting; the Rust crate does not.
                let mut literal = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '\\' && chars.peek() == Some(&'E') {
                        chars.next();
                        closed = true;
                        break;
                    }
                    literal.push(c);
                }
                if !closed {
                    return Err(invalid());
                }
                translated.push_str(&regex::escape(&literal));
                continue;
            }
            let expansion = if in_class {
                match next {
                    'd' => Some("0-9"),
                    'w' => Some("0-9A-Za-z_"),
                    's' => Some(r"\t\n\f\r "),
                    // A negated ASCII class cannot be inserted into a larger
                    // character class safely. Reject instead of a false match.
                    'D' | 'W' | 'S' => return Err(invalid()),
                    _ => None,
                }
            } else {
                match next {
                    'd' => Some("[0-9]"),
                    'D' => Some("[^0-9]"),
                    'w' => Some("[0-9A-Za-z_]"),
                    'W' => Some("[^0-9A-Za-z_]"),
                    's' => Some(r"[\t\n\f\r ]"),
                    'S' => Some(r"[^\t\n\f\r ]"),
                    'b' => Some(r"(?-u:\b)"),
                    'B' => Some(r"(?-u:\B)"),
                    _ => None,
                }
            };
            if let Some(expansion) = expansion {
                translated.push_str(expansion);
            } else {
                translated.push('\\');
                translated.push(next);
            }
            continue;
        }
        if ch == '[' && !in_class {
            // A class containing only one negative ASCII shorthand can be
            // represented exactly. Mixed complements remain rejected below.
            let look: Vec<char> = chars.clone().take(4).collect();
            let pair = if look.len() >= 3 && look[0] == '\\' && look[2] == ']' {
                Some((false, look[1], 3))
            } else if look.len() >= 4 && look[0] == '^' && look[1] == '\\' && look[3] == ']' {
                Some((true, look[2], 4))
            } else {
                None
            };
            if let Some((inverted, code, count)) = pair {
                let exact = match (inverted, code) {
                    (false, 'D') | (true, 'd') => Some("[^0-9]"),
                    (true, 'D') | (false, 'd') => Some("[0-9]"),
                    (false, 'W') | (true, 'w') => Some("[^0-9A-Za-z_]"),
                    (true, 'W') | (false, 'w') => Some("[0-9A-Za-z_]"),
                    (false, 'S') | (true, 's') => Some(r"[^\t\n\f\r ]"),
                    (true, 'S') | (false, 's') => Some(r"[\t\n\f\r ]"),
                    _ => None,
                };
                if let Some(exact) = exact {
                    translated.push_str(exact);
                    for _ in 0..count {
                        chars.next();
                    }
                    continue;
                }
            }
        }
        if ch == '(' && !in_class && chars.peek() == Some(&'?') {
            let flags: String = chars
                .clone()
                .skip(1)
                .take_while(|c| !matches!(c, ')' | ':' | '<'))
                .take(20)
                .collect();
            // Go RE2 accepts i/m/s/U and named captures; Rust-only
            // verbose or Unicode flags must not change printer eligibility.
            if flags.chars().any(|flag| matches!(flag, 'x' | 'u')) {
                return Err(invalid());
            }
        }
        if ch == '{' && !in_class {
            let rest: String = chars.clone().take(20).collect();
            if let Some(end) = rest.find('}') {
                let body = &rest[..end];
                // Go accepts {,2} as literal braces, not a repetition.
                if body.starts_with(',') && body[1..].bytes().all(|b| b.is_ascii_digit()) {
                    translated.push_str(r"\{");
                    translated.push_str(body);
                    translated.push_str(r"\}");
                    for _ in 0..=end {
                        chars.next();
                    }
                    continue;
                }
                // RE2 bounds repetitions to 1000; Rust permits larger ones.
                if body.split(',').any(|part| {
                    !part.is_empty()
                        && part.bytes().all(|b| b.is_ascii_digit())
                        && part.parse::<u64>().map_or(true, |value| value > 1000)
                }) {
                    return Err(invalid());
                }
            }
        }
        if in_class && ch == '-' && chars.peek() == Some(&'-') && !at_class_start {
            // Go reports an invalid range for [a--b], while Rust would treat
            // -- as character-class set subtraction.
            return Err(invalid());
        }
        if ch == '[' && in_class {
            // In Go RE2, an unescaped '[' inside a class is a literal '['.
            // Rust regex instead opens a nested class, which can silently
            // mark an incompatible printer as compatible. POSIX classes are
            // the only supported nested-looking '[...]' syntax in Go.
            if chars.peek() == Some(&':') {
                let look: Vec<char> = chars.clone().take(32).collect();
                if let Some(end) = look.windows(2).position(|pair| pair == [':', ']']) {
                    let name: String = look[1..end].iter().collect();
                    let name = name.strip_prefix('^').unwrap_or(&name);
                    const POSIX: &[&str] = &[
                        "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower",
                        "print", "punct", "space", "upper", "word", "xdigit",
                    ];
                    if !POSIX.contains(&name) {
                        return Err(invalid());
                    }
                    translated.push('[');
                    // Copy the complete POSIX token, including the ':]'.
                    // Its closing bracket must not close the outer class.
                    for _ in 0..end + 2 {
                        translated.push(chars.next().ok_or_else(invalid)?);
                    }
                    at_class_start = false;
                    continue;
                }
                return Err(invalid());
            }
            translated.push_str(r"\[");
            at_class_start = false;
            continue;
        }
        if ch == '[' {
            in_class = true;
            at_class_start = true;
        } else if ch == ']' {
            in_class = false;
            at_class_start = false;
        } else if in_class && ch != '^' {
            at_class_start = false;
        }
        if in_class && ch == '&' && chars.peek() == Some(&'&') {
            // Go treats && as two literals; Rust uses set intersection.
            translated.push_str(r"\&\&");
            chars.next();
            continue;
        }
        translated.push(ch);
    }
    Ok(translated)
}

fn compile_prusa_regex(pattern: &str) -> Result<regex::Regex, String> {
    let compatible = go_regex_pattern(pattern)?;
    regex::RegexBuilder::new(&compatible)
        .size_limit(MAX_PRUSA_REGEX_COMPILED_BYTES)
        .build()
        .map_err(|_| "compatibility condition has an invalid regular expression".to_owned())
}

/// Compares numerically when both sides parse as numbers, so `0.2` and `0.20`
/// match, and compares text otherwise.
pub(crate) fn prusa_values_equal(a: &str, b: &str) -> bool {
    match (parse_prusa_number(a), parse_prusa_number(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

/// Parses an operand the way Go's `strconv.ParseFloat` does: hexadecimal floats
/// such as `0x1p-2` are numbers, and a sign may precede only `Inf` and `Infinity`,
/// so a signed `nan` compares as text. Numeric underscores follow Go's
/// digit-separator placement rules; malformed separators compare as text.
fn parse_prusa_number(text: &str) -> Option<f64> {
    // Go strconv.ParseFloat also accepts digit separators in valid positions.
    let cleaned = if text.contains('_') {
        Some(strip_go_numeric_underscores(text)?)
    } else {
        None
    };
    let text = cleaned.as_deref().unwrap_or(text);
    let signed = text.starts_with(['+', '-']);
    let unsigned = if signed { &text[1..] } else { text };
    let prefix = unsigned.as_bytes().get(..2);
    if prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"0x")) {
        return parse_prusa_hex_float(text);
    }
    if signed && unsigned.eq_ignore_ascii_case("nan") {
        return None;
    }
    let value = text.parse::<f64>().ok()?;
    // Go's ParseFloat reports an overflowing literal as an error, so it compares
    // as text. Rust returns infinity for it, so only spelled-out infinity counts.
    let spelled_infinity =
        unsigned.eq_ignore_ascii_case("inf") || unsigned.eq_ignore_ascii_case("infinity");
    (!value.is_infinite() || spelled_infinity).then_some(value)
}

/// Go permits one underscore between digits, and immediately after 0x/0X.
/// It does not permit separators next to the decimal point or exponent marker.
fn strip_go_numeric_underscores(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let sign = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let hex = bytes
        .get(sign..sign + 2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"0x"));
    let exponent_at = bytes
        .iter()
        .enumerate()
        .skip(sign + usize::from(hex) * 2)
        .find(|(_, byte)| {
            if hex {
                **byte == b'p' || **byte == b'P'
            } else {
                **byte == b'e' || **byte == b'E'
            }
        })
        .map(|(index, _)| index)
        .unwrap_or(bytes.len());
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'_' {
            continue;
        }
        let next = *bytes.get(index + 1)?;
        if hex && index == sign + 2 {
            if !next.is_ascii_hexdigit() {
                return None;
            }
            continue;
        }
        let prev = *bytes.get(index.checked_sub(1)?)?;
        let valid = if hex && index < exponent_at {
            prev.is_ascii_hexdigit() && next.is_ascii_hexdigit()
        } else {
            prev.is_ascii_digit() && next.is_ascii_digit()
        };
        if !valid {
            return None;
        }
    }
    Some(text.replace('_', ""))
}

/// Parses Go's hexadecimal float form: `0x` digits with an optional `.` and a
/// mandatory `p` exponent, such as `0x1.8p1`. The mantissa keeps 128 bits, and
/// any lower digits are folded into a sticky bit, so the final rounding matches a
/// correctly rounded conversion. Returns `None` on overflow, as Go does.
fn parse_prusa_hex_float(text: &str) -> Option<f64> {
    let (sign, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let body = unsigned.get(2..)?;
    let (mantissa, exponent) = body.split_once(['p', 'P'])?;
    let exponent: i64 = exponent.parse().ok()?;
    let (int_digits, frac_digits) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int_digits.is_empty() && frac_digits.is_empty() {
        return None;
    }
    let mut bits: u128 = 0;
    let mut dropped: i64 = 0;
    let mut sticky = false;
    for byte in int_digits.bytes().chain(frac_digits.bytes()) {
        let digit = u128::from((byte as char).to_digit(16)?);
        if bits >> 124 == 0 {
            bits = bits * 16 + digit;
        } else {
            dropped += 1;
            sticky |= digit != 0;
        }
    }
    let frac_len = frac_digits.len() as i64;
    let scale = exponent
        .saturating_sub(4 * frac_len)
        .saturating_add(4 * dropped)
        .clamp(-4000, 4000);
    let mut value = (bits | u128::from(sticky)) as f64;
    // Scale in steps that stay inside f64's normal range, so 2^-1074 is reachable.
    let mut remaining = scale;
    while remaining != 0 {
        let step = remaining.clamp(-512, 512);
        value *= 2f64.powi(step as i32);
        remaining -= step;
    }
    value.is_finite().then_some(sign * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(pairs: &[(&str, &str)]) -> PrusaSection {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn evaluates_real_vendor_forms() {
        let printer = section(&[
            ("printer_model", "X-Plus 4"),
            ("nozzle_diameter", "0.4;0.4"),
        ]);
        let cases: Vec<(&str, bool)> = vec![
            (r#"printer_model=="X-Plus 4""#, true),
            (r#"printer_model=="Q1 Pro""#, false),
            (r#"printer_model!="Q1 Pro""#, true),
            (
                r#"printer_model=="X-Plus 4" and nozzle_diameter[0]==0.4"#,
                true,
            ),
            (
                r#"printer_model=="X-Plus 4" and nozzle_diameter[0]==0.2"#,
                false,
            ),
            ("nozzle_diameter[0]==0.40", true),
            ("nozzle_diameter[1]==0.4", true),
            ("nozzle_diameter[9]==\"\"", true),
            (
                r#"printer_model=="Q1 Pro" or nozzle_diameter[0]==0.4"#,
                true,
            ),
            (
                r#"(printer_model=="Q1 Pro" or printer_model=="X-Plus 4") and nozzle_diameter[0]==0.4"#,
                true,
            ),
            ("missing_setting==\"\"", true),
            ("printer_model=~/^X-Plus 4$/", true),
            ("printer_model=~/PRINTER_VENDOR_PRUSA3D/", false),
            ("printer_model!~/Q1/ and nozzle_diameter[0]==0.4", true),
            (r"printer_model=~/X\/Plus/", false),
            ("printer_model=~/.*X-Plus.*/ and num_extruders==1", false),
            ("printer_model[0]==\"X-Plus 4\"", true),
            ("printer_model[+0]==\"X-Plus 4\"", true),
        ];
        for (expr, want) in cases {
            let got = evaluate_prusa_condition(expr, &printer)
                .unwrap_or_else(|error| panic!("{expr}: unexpected error {error}"));
            assert_eq!(got, want, "{expr}");
        }
    }

    #[test]
    fn fails_closed_on_unsupported_syntax() {
        let printer = section(&[("printer_model", "X-Plus 4")]);
        let oversized = format!(
            r#"printer_model=="{}""#,
            "x".repeat(MAX_PRUSA_CONDITION_BYTES)
        );
        let cases: Vec<&str> = vec![
            "",
            "printer_model=~/X-Plus [",
            r#"printer_model=~"X-Plus""#,
            "printer_model=~/unterminated",
            r#"printer_model=~ "X""#,
            "printer_model!~/[a-z/",
            r#"printer_model="X-Plus 4"#,
            r#"printer_model=="X-Plus 4" and"#,
            r#"printer_model == "X" trailing"#,
            r#"printer_model=="X-Plus 4") "#,
            r#"(printer_model=="X-Plus 4""#,
            "printer_model[99999]==\"\"",
            r#"printer_model>="X""#,
            "printer_model",
            r#"printer_model=="X" or 1"#,
            r#"system("ls")"#,
            "printer_model[64]==\"\"",
            "printer_model[1x]==\"\"",
            "printer_model[1==\"\"",
            "1-2==\"\"",
            oversized.as_str(),
        ];
        for expr in cases {
            assert!(
                evaluate_prusa_condition(expr, &printer).is_err(),
                "{expr:?} accepted"
            );
        }
    }

    #[test]
    fn bounds_token_count_and_regex_length() {
        let printer = PrusaSection::new();
        // 65 terms of `a==""` joined by ` or ` make 259 tokens; 64 make 255.
        let within = vec!["a==\"\""; 64].join(" or ");
        assert!(evaluate_prusa_condition(&within, &printer).is_ok());
        let over = vec!["a==\"\""; 65].join(" or ");
        assert!(evaluate_prusa_condition(&over, &printer).is_err());

        let at_limit = format!("a=~/{}/", "x".repeat(MAX_PRUSA_REGEX_BYTES));
        assert!(evaluate_prusa_condition(&at_limit, &printer).is_ok());
        let over_limit = format!("a=~/{}/", "x".repeat(MAX_PRUSA_REGEX_BYTES + 1));
        assert!(evaluate_prusa_condition(&over_limit, &printer).is_err());
    }

    #[test]
    fn matches_go_re2_ascii_shorthands_and_literal_operators() {
        // Go's regexp.Compile was executed on the same cases independently.
        let cases = [
            (r"^\d+$", "٣", false),
            (r"^\d+$", "123", true),
            (r"\w+", "é", false),
            (r"\w+", "abc", true),
            (r"X\sPlus", "X\u{00a0}Plus", false),
            (r"X\sPlus", "X Plus", true),
            (r"\bX", "éX", true),
            (r"[a&&b]", "a", true),
            (r"[a&&b]", "&", true),
            (r"[\w]+", "é", false),
            (r"\s", "\u{000b}", false),
            (r"\s", "\u{00a0}", false),
            (r"\S", "\u{00a0}", true),
            (r"\W", "é", true),
            (r"[\D]", "é", true),
            (r"[\W]", "é", true),
            (r"[\S]", "é", true),
            (r"[^\D]", "3", true),
            (r"\Qx\E", "x", true),
            (r"a{,2}", "a{,2}", true),
            (r"\b{start}X", "a{start}X", true),
            (r"(?U)a.*b", "a___b___b", true),
            (r"(?<n>x)", "x", true),
            (r"\pL", "é", true),
            (r"[[:alpha:]]", "é", false),
        ];
        for (pattern, model, expected) in cases {
            let section = section(&[("printer_model", model)]);
            let expression = format!("printer_model=~/{pattern}/");
            assert_eq!(
                evaluate_prusa_condition(&expression, &section),
                Ok(expected),
                "{expression:?} with {model:?}"
            );
        }
    }

    #[test]
    fn re2_literal_open_bracket_inside_class_never_enables_incompatible_printers() {
        // Independently tested with native/presets/prusa_condition.go's
        // regexp.Compile on each exact pattern/value pair below.
        let cases = [
            (r"[a[]", "a", true),
            (r"[a[]", "[", true),
            (r"[a[]", "]", false),
            (r"[[]", "[", true),
            (r"[[]", "a", false),
            (r"[a[b]", "b", true),
            (r"[a[b]", "[", true),
            // Rust's nested-class interpretation previously matched "a",
            // "b" or "]", even though Go's pattern does not.
            (r"[a[]b]]", "a", false),
            (r"[a[]b]]", "b", false),
            (r"[a[]b]]", "]", false),
            (r"[[:alpha:]]", "a", true),
            (r"[[:alpha:]]", "é", false),
            (r"[[:digit:]a]", "5", true),
            (r"[[:digit:]a]", "a", true),
            (r"[^[]", "x", true),
            (r"[^[]", "[", false),
            (r"[[:^digit:]]", "a", true),
            (r"[[:^digit:]]", "3", false),
            (r"[[:word:]]", "_", true),
            (r"[[:word:]]", "é", false),
        ];
        for (pattern, model, expected) in cases {
            let section = section(&[("printer_model", model)]);
            let expression = format!("printer_model=~/{pattern}/");
            assert_eq!(
                evaluate_prusa_condition(&expression, &section),
                Ok(expected),
                "{expression:?} with {model:?}"
            );
        }
    }

    #[test]
    fn rejects_rust_only_go_invalid_regex_extensions() {
        let section = section(&[("printer_model", "XPlus")]);
        for pattern in [r"(?x)X Plus", r"\u0058", r"(?-u)\w+", "a{1001}", "[a--b]"] {
            let expression = format!("printer_model=~/{pattern}/");
            assert!(
                evaluate_prusa_condition(&expression, &section).is_err(),
                "Go rejects {pattern:?}, Rust must also reject"
            );
        }
    }

    #[test]
    fn compares_numbers_only_when_both_sides_parse() {
        assert!(prusa_values_equal("0.2", "0.20"));
        assert!(prusa_values_equal("inf", "Infinity"));
        assert!(!prusa_values_equal("NaN", "NaN"));
        // Overflowing literals are not numbers in Go, so they compare as text.
        assert!(prusa_values_equal("1e400", "1e400"));
        assert!(!prusa_values_equal("1e400", "1e401"));
        assert!(!prusa_values_equal("X-Plus 4", "x-plus 4"));
    }

    #[test]
    fn compares_hex_floats_numerically_like_go() {
        assert!(prusa_values_equal("0x1p-2", "0.25"));
        assert!(prusa_values_equal("0X1.8P1", "3"));
        assert!(prusa_values_equal("-0x1p0", "-1"));
        assert!(prusa_values_equal("+0x1p+1", "2"));
        assert!(prusa_values_equal("0x0p0", "-0"));
        // A long mantissa keeps its value: 0x1 followed by 40 zeros, scaled by 2^-160.
        assert!(prusa_values_equal(
            &format!("0x1{}p-160", "0".repeat(40)),
            "1"
        ));
        // The smallest subnormal, reachable only through the stepped scaling.
        assert!(prusa_values_equal("0x1p-1074", "5e-324"));
        // Go requires the p exponent, so these are text and only equal as text.
        assert!(!prusa_values_equal("0x1.8", "1.5"));
        assert!(prusa_values_equal("0x1.8", "0x1.8"));
        // Overflow is not a number in Go, so the text comparison decides.
        assert!(!prusa_values_equal("0x1p1024", "0x1p1025"));
        assert!(prusa_values_equal("0x1p1024", "0x1p1024"));
        assert!(!prusa_values_equal("0xp1", "1"));
        assert!(!prusa_values_equal("0x.p1", "0"));
    }

    #[test]
    fn signed_nan_compares_as_text() {
        assert!(prusa_values_equal("-nan", "-nan"));
        assert!(!prusa_values_equal("-nan", "NaN"));
        // Unsigned NaN is still a number, and NaN never equals NaN.
        assert!(!prusa_values_equal("NaN", "NaN"));
    }

    #[test]
    fn go_numeric_underscore_placements_are_source_locked() {
        assert!(prusa_values_equal("1_0", "10"));
        assert!(prusa_values_equal("1e1_0", "1e10"));
        assert!(prusa_values_equal("0x_1p0", "1"));
        assert!(prusa_values_equal("0x1e_2p0", "482"));
        assert!(prusa_values_equal("-0x_1p0", "-1"));
        assert!(!prusa_values_equal("1e_2", "100"));
        assert!(!prusa_values_equal("1_.0", "1"));
        assert!(!prusa_values_equal("0x1_p0", "1"));
        assert!(!prusa_values_equal("0x1p_0", "1"));
    }
}
