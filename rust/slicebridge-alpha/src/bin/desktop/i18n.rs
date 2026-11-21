//! 1:1 main-window translations exported from main/native/frontend/dist/app.js.
//! The JSON is embedded into the Rust binary: no JS runtime or WebView.
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Locale {
    He,
    En,
}

impl Locale {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::He => "he",
            Self::En => "en",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        match code.trim() {
            "he" => Some(Self::He),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    fn path() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("APPDATA").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        // Keep experimental settings away from the installed stable application.
        base.map(|base| base.join("SliceBridge/GPUI-Alpha/3mfBridgeLanguage.txt"))
    }

    pub(super) fn load() -> Self {
        if let Some(saved) = Self::path().and_then(|path| std::fs::read_to_string(path).ok()) {
            if let Some(locale) = Self::from_code(&saved) {
                return locale;
            }
        }
        // Wails uses navigator.language; Finder-launched macOS apps need the
        // first AppleLanguages preference, not a shell-specific LANG fallback.
        #[cfg(target_os = "macos")]
        if let Some(code) = super::desktop_macos_language::preferred_language() {
            if let Some(locale) = Self::from_code(code) {
                return locale;
            }
        }
        for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(language) = std::env::var(key) {
                let normalized = language.to_ascii_lowercase();
                if normalized.starts_with("he") || normalized.starts_with("iw") {
                    return Self::He;
                }
                if normalized.starts_with("en") {
                    return Self::En;
                }
            }
        }
        Self::En
    }

    pub(super) fn save(self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("no config directory"))?;
        std::fs::create_dir_all(path.parent().expect("language file has parent"))?;
        std::fs::write(path, self.code())
    }

    pub(super) fn text(self, key: &str) -> String {
        static TRANSLATIONS: OnceLock<Value> = OnceLock::new();
        let all = TRANSLATIONS.get_or_init(|| {
            serde_json::from_str(include_str!("translations.json"))
                .expect("checked-in baseline translations must be valid JSON")
        });
        all[self.code()][key].as_str().unwrap_or(key).to_owned()
    }

    pub(super) fn rtl(self) -> bool {
        self == Self::He
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_locked_languages_have_identical_keys() {
        let all: Value = serde_json::from_str(include_str!("translations.json")).unwrap();
        let he = all["he"].as_object().unwrap();
        let en = all["en"].as_object().unwrap();
        assert!(he.len() >= 110);
        assert_eq!(he.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
        assert_eq!(Locale::He.text("workspaceTitle"), "סביבת המרה");
        assert_eq!(Locale::En.text("workspaceTitle"), "Conversion workspace");
        assert_eq!(
            Locale::He.text("queueFootnote"),
            "עד 200 קובצי 3MF, כולל תיקיות משנה."
        );
        assert_eq!(Locale::En.text("resultsTitle"), "Conversion results");
        assert_eq!(
            Locale::He.text("outputFolderMissing"),
            "תיקיית התוצרים אינה קיימת עוד"
        );
        assert_eq!(
            Locale::En.text("outputFolderMissing"),
            "Output folder no longer exists"
        );
        assert_eq!(
            Locale::He.text("outputFolderOpenError"),
            "לא ניתן לפתוח את תיקיית התוצרים"
        );
        assert_eq!(
            Locale::En.text("outputFolderOpenError"),
            "Could not open output folder"
        );
    }

    #[test]
    fn locale_direction_and_unknown_translation() {
        assert!(Locale::He.rtl());
        assert!(!Locale::En.rtl());
        assert_eq!(Locale::He.text("nonexistent-key"), "nonexistent-key");
        assert_eq!(Locale::from_code("he\n"), Some(Locale::He));
    }
}
