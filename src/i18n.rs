//! Explicit UI catalog. Device names, paths and diagnostic payloads stay unchanged.
use serde::{Deserialize, Serialize};
use std::sync::{
    OnceLock,
    atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "it")]
    Italian,
}
impl Language {
    pub const ALL: [Self; 6] = [
        Self::English,
        Self::Russian,
        Self::French,
        Self::German,
        Self::Spanish,
        Self::Italian,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
            Self::French => "Français",
            Self::German => "Deutsch",
            Self::Spanish => "Español",
            Self::Italian => "Italiano",
        }
    }
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
            Self::French => "fr",
            Self::German => "de",
            Self::Spanish => "es",
            Self::Italian => "it",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
    }
    pub fn text(self, english: &str) -> &str {
        if self == Self::English {
            return english;
        }
        catalog()
            .iter()
            .find(|row| row[0] == english)
            .map_or(english, |row| row[self.index()].as_str())
    }
}
fn catalog() -> &'static Vec<[String; 6]> {
    static CATALOG: OnceLock<Vec<[String; 6]>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        include_str!("locales.tsv")
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let row: Vec<_> = line
                    .split('|')
                    .map(|value| {
                        value
                            .replace("\\r", "\r")
                            .replace("\\n", "\n")
                            .replace("\\t", "\t")
                    })
                    .collect();
                row.try_into()
                    .expect("each catalog row has exactly six languages")
            })
            .collect()
    })
}
static LANGUAGE: AtomicU8 = AtomicU8::new(0);
pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}
pub fn tr(english: &str) -> &str {
    Language::ALL[LANGUAGE.load(Ordering::Relaxed) as usize].text(english)
}
pub fn message(template: &str, values: &[(&str, String)]) -> String {
    let mut result = tr(template).to_owned();
    for (name, value) in values {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

/// Translate a known product error prefix while retaining Windows codes, sources and paths.
/// Only operation diagnostics call this function; it is never applied to user data fields.
pub fn diagnostic(english: &str) -> String {
    let language = Language::ALL[LANGUAGE.load(Ordering::Relaxed) as usize];
    diagnostic_in(language, english)
}
fn diagnostic_in(language: Language, english: &str) -> String {
    if let Some(row) = catalog()
        .iter()
        .filter(|row| english.starts_with(&row[0]))
        .filter(|row| {
            let suffix = &english[row[0].len()..];
            suffix.is_empty() || suffix.starts_with(": ") || suffix.starts_with(" (")
        })
        .max_by_key(|row| row[0].len())
    {
        if let Some(detail) = english[row[0].len()..].strip_prefix(": ") {
            return format!(
                "{}: {}",
                language.text(&row[0]),
                diagnostic_in(language, detail)
            );
        }
        return format!("{}{}", language.text(&row[0]), &english[row[0].len()..]);
    }
    english.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .map(|part| part.split('}').next().unwrap())
            .collect()
    }
    #[test]
    fn every_message_has_six_complete_placeholder_compatible_variants() {
        let mut keys = BTreeSet::new();
        for row in catalog() {
            assert!(keys.insert(&row[0]), "duplicate message: {}", row[0]);
            for translated in &row[1..] {
                assert!(
                    !translated.trim().is_empty(),
                    "empty translation: {}",
                    row[0]
                );
                assert_eq!(
                    placeholders(&row[0]),
                    placeholders(translated),
                    "placeholder mismatch: {}",
                    row[0]
                );
            }
        }
    }
    #[test]
    fn language_selection_preserves_unknown_device_and_diagnostic_data() {
        for language in Language::ALL {
            assert_eq!(
                language.text("D:\\User's drive\\photo.jpg"),
                "D:\\User's drive\\photo.jpg"
            );
            assert_eq!(
                language.text("USBSTOR\\Unknown-serial"),
                "USBSTOR\\Unknown-serial"
            );
            assert!(!language.text("Safely eject").is_empty());
        }
    }
    #[test]
    fn veto_reason_is_translated_without_rewriting_windows_source_or_code() {
        let source = "A Windows application refused removal: D:\\Data\\Editor.exe (Configuration Manager 0x00000017, veto 3)";
        let translated = diagnostic_in(Language::Russian, source);
        assert!(translated.starts_with("Приложение Windows запретило извлечение: "));
        assert!(
            translated.ends_with("D:\\Data\\Editor.exe (Configuration Manager 0x00000017, veto 3)")
        );
    }
    #[test]
    fn literal_interface_messages_are_present_in_the_catalog() {
        let source = include_str!("ui.rs");
        for marker in ["tr(\"", "message(\""] {
            for rest in source.split(marker).skip(1) {
                let mut escaped = false;
                let end = rest
                    .char_indices()
                    .find_map(|(index, character)| {
                        if character == '"' && !escaped {
                            return Some(index);
                        }
                        escaped = character == '\\' && !escaped;
                        None
                    })
                    .unwrap();
                let literal = format!("\"{}\"", &rest[..end]);
                let english: String = serde_json::from_str(&literal).unwrap();
                assert!(
                    catalog().iter().any(|row| row[0] == english),
                    "missing UI message: {english}"
                );
            }
        }
    }
}
