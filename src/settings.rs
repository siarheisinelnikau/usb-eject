use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schema: u32,
    pub check_updates_on_startup: bool,
    pub aliases: BTreeMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self { Self { schema: 1, check_updates_on_startup: true, aliases: BTreeMap::new() } }
}
impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(bytes) => {
                let value: Self = serde_json::from_slice(&bytes).map_err(|e| format!("Settings are invalid; original file preserved: {e}"))?;
                if value.schema != 1 { return Err("Unsupported settings schema; original file preserved".into()); }
                Ok(value)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read settings: {e}")),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("Settings directory is missing")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| format!("Cannot stage settings: {e}"))?;
        use std::io::Write;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace(&temp, path)
        })();
        if result.is_err() { let _ = fs::remove_file(&temp); }
        result.map_err(|e| format!("Cannot save settings: {e}"))
    }
}
pub fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("UsbEject")).ok_or("LOCALAPPDATA is unavailable".into())
}
pub fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)] {
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: terminated UTF-16 paths stay alive for the synchronous call.
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 { return Err(std::io::Error::last_os_error()); }
        Ok(())
    }
    #[cfg(not(windows))] { fs::rename(from, to) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_roundtrip_and_corrupt_file_preservation() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".test-output").join(format!("settings-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let mut value = Settings::default();
        value.check_updates_on_startup = false;
        value.aliases.insert("sample".into(), "Backup".into());
        value.save(&path).unwrap();
        assert!(!Settings::load(&path).unwrap().check_updates_on_startup);
        assert_eq!(Settings::load(&path).unwrap().aliases["sample"], "Backup");
        value.save(&path).unwrap();
        fs::write(&path, b"invalid fixture").unwrap();
        assert!(Settings::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid fixture");
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
