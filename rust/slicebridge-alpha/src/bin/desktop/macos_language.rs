//! The installed Wails UI defaults to navigator.language when no language was saved.
//! On macOS, the preferred UI language comes from AppleLanguages, which often
//! differs from the shell's LANG/LC_* variables (especially Finder-launched apps).
//! Read-only, no AppleScript, shell interpolation, or installed-app preference writes.
#[cfg(target_os = "macos")]
pub(crate) fn preferred_language() -> Option<&'static str> {
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    first_supported_language(std::str::from_utf8(&output.stdout).ok()?)
}

fn first_supported_language(raw: &str) -> Option<&'static str> {
    // "defaults read" prints a plist array: (\n    "he-IL",\n    "en-US"\n).
    // Only the FIRST preference drives navigator.language. Do not choose a
    // later Hebrew language when the user's first language is unsupported.
    let first = raw
        .split(',')
        .map(|entry| {
            entry.trim_matches(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | '"'))
        })
        .find(|entry| !entry.is_empty())?;
    let code = first
        .split(|ch| ch == '-' || ch == '_')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match code.as_str() {
        "he" | "iw" => Some("he"),
        "en" => Some("en"),
        // Wails only translates Hebrew and English: an otherwise valid French,
        // Arabic, etc. FIRST browser language displays the English UI; do not
        // substitute the later Hebrew system option or shell's LANG value.
        unknown
            if (2..=3).contains(&unknown.len())
                && unknown.bytes().all(|byte| byte.is_ascii_alphabetic()) =>
        {
            Some("en")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::first_supported_language;

    #[test]
    fn first_macos_language_controls_initial_hebrew_or_english() {
        assert_eq!(
            first_supported_language("(\n    \"he-IL\",\n    \"en-US\"\n)"),
            Some("he")
        );
        assert_eq!(
            first_supported_language("(\n    \"en-IL\",\n    \"he-IL\"\n)"),
            Some("en")
        );
        assert_eq!(
            first_supported_language("(\"iw_IL\", \"en-US\")"),
            Some("he")
        );
    }

    #[test]
    fn unknown_first_language_does_not_select_later_hebrew() {
        assert_eq!(
            first_supported_language("(\"fr-FR\", \"he-IL\")"),
            Some("en")
        );
        assert_eq!(first_supported_language("()"), None);
        assert_eq!(first_supported_language(""), None);
        assert_eq!(first_supported_language("(\"hello-IL\")"), None);
    }
}
