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

fn compile_prusa_regex(pattern: &str) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern)
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
