pub mod storage;
pub mod installer;

pub fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
pub fn from_wide(text: &[u16]) -> String { String::from_utf16_lossy(&text[..text.iter().position(|&x| x == 0).unwrap_or(text.len())]) }
