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
}
