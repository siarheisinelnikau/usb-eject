use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}, time::Duration};

pub const REPOSITORY: &str = "siarheisinelnikau/usb-eject";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_EXE: u64 = 100 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Asset { pub name: String, pub browser_download_url: String, pub size: u64 }
#[derive(Clone, Debug, Deserialize)]
pub struct Release { pub tag_name: String, pub draft: bool, pub prerelease: bool, pub assets: Vec<Asset> }
#[derive(Clone, Debug)]
pub struct Available { pub version: Version, pub executable: Asset, pub checksums: Asset }

pub fn select_release(release: Release, installed: &str) -> Result<Option<Available>, String> {
    if release.draft || release.prerelease { return Ok(None); }
    let version = Version::parse(release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name)).map_err(|_| "Release has an invalid version")?;
    if !version.pre.is_empty() || !version.build.is_empty() { return Ok(None); }
    if version <= Version::parse(installed).map_err(|e| e.to_string())? { return Ok(None); }
    if !cfg!(target_arch = "x86_64") { return Err("Automatic installation currently supports Windows x64 only".into()); }
    let name = format!("UsbEject-v{version}.exe");
    let executable = release.assets.iter().find(|a| a.name == name).cloned().ok_or("Release has no compatible standalone executable")?;
    let checksums = release.assets.iter().find(|a| a.name == "SHA256SUMS").cloned().ok_or("Release has no SHA256SUMS; automatic installation is unavailable")?;
    for asset in [&executable, &checksums] {
        let expected = format!("https://github.com/{REPOSITORY}/releases/download/{}/{name}", release.tag_name, name = asset.name);
        if asset.browser_download_url != expected { return Err("Release asset is outside the canonical repository".into()); }
    }
    if executable.size < 512 || executable.size > MAX_EXE || checksums.size > 64 * 1024 { return Err("Release asset has an invalid size".into()); }
    Ok(Some(Available { version, executable, checksums }))
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder().https_only(true).timeout_global(Some(Duration::from_secs(timeout))).max_redirects(5).user_agent(format!("UsbEject/{VERSION}")).build().into()
}

fn get_small(url: &str, limit: u64) -> Result<String, String> {
    let mut response = agent(25).get(url).header("Accept", "application/vnd.github+json").call().map_err(|e| format!("Update source unavailable (unpublished repository/release, network failure or rate limit): {e}"))?;
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit { return Err("Update response exceeds the size limit".into()); }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

pub fn check() -> Result<Option<Available>, String> {
    let json = get_small(&format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"), 1024 * 1024)?;
    let release = serde_json::from_str(&json).map_err(|e| format!("Invalid release metadata: {e}"))?;
    select_release(release, VERSION)
}

pub fn checksum_for(text: &str, name: &str) -> Result<String, String> {
    let mut found = None;
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1].trim_start_matches('*') == name {
            if found.is_some() { return Err("Duplicate executable checksum".into()); }
            if parts[0].len() != 64 || !parts[0].bytes().all(|b| b.is_ascii_hexdigit()) { return Err("Invalid SHA-256 checksum".into()); }
            found = Some(parts[0].to_ascii_lowercase());
        }
    }
    found.ok_or("Executable checksum is missing".into())
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn verify_pe_x64(path: &Path) -> Result<(), String> {
    use std::io::{Seek, SeekFrom};
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|e| e.to_string())?;
    if &dos[..2] != b"MZ" { return Err("Downloaded asset is not a Windows executable".into()); }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if offset > 1024 * 1024 { return Err("Invalid executable header".into()); }
    file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut header = [0u8; 6];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"PE\0\0" || u16::from_le_bytes([header[4], header[5]]) != 0x8664 { return Err("Update is not a Windows x64 executable".into()); }
    Ok(())
}

/// Only called after the application's Download and install action.
pub fn download(available: &Available, root: &Path, cancel: &AtomicBool, progress: impl Fn(u64, u64)) -> Result<(PathBuf, String), String> {
    let checksum_text = get_small(&available.checksums.browser_download_url, 64 * 1024)?;
    let expected = checksum_for(&checksum_text, &available.executable.name)?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let folder = root.join(format!("{}-{}-{unique}", available.version, std::process::id()));
    fs::create_dir(&folder).map_err(|e| format!("Cannot create unique update staging directory: {e}"))?;
    let path = folder.join(&available.executable.name);
    let result = (|| {
        let mut response = agent(180).get(&available.executable.browser_download_url).call().map_err(|e| format!("Download failed: {e}"))?;
        let mut reader = response.body_mut().as_reader();
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0;
        let mut last_percent = 101;
        loop {
            if cancel.load(Ordering::Acquire) { return Err("Update download cancelled".into()); }
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            total += n as u64;
            if total > available.executable.size || total > MAX_EXE { return Err("Download is larger than declared".into()); }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            let percent = total * 100 / available.executable.size;
            if percent != last_percent { progress(total, available.executable.size); last_percent = percent; }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if total != available.executable.size { return Err("Download is incomplete".into()); }
        if hash_file(&path)? != expected { return Err("Checksum verification failed; installation was not changed".into()); }
        verify_pe_x64(&path)?;
        Ok((path.clone(), expected))
    })();
    if result.is_err() { let _ = fs::remove_file(&path); let _ = fs::remove_dir(&folder); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(version: &str) -> Release {
        let name = format!("UsbEject-v{version}.exe");
        Release { tag_name: format!("v{version}"), draft: false, prerelease: false, assets: [name, "SHA256SUMS".into()].into_iter().map(|name| Asset { browser_download_url: format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"), name, size: 4096 }).collect() }
    }
    #[test]
    fn semantic_versions_and_stable_channel() {
        assert!(select_release(release("0.10.0"), "0.9.0").unwrap().is_some());
        assert!(select_release(release("0.1.0"), "0.1.0").unwrap().is_none());
        let mut draft = release("0.2.0"); draft.draft = true;
        assert!(select_release(draft, "0.1.0").unwrap().is_none());
        assert!(select_release(release("0.2.0-beta.1"), "0.1.0").unwrap().is_none());
    }
    #[test]
    fn unsafe_assets_and_missing_checksums_are_rejected() {
        let mut data = release("0.2.0");
        data.assets[0].browser_download_url = "https://example.org/UsbEject.exe".into();
        assert!(select_release(data, "0.1.0").is_err());
        let mut data = release("0.2.0"); data.assets.pop();
        assert!(select_release(data, "0.1.0").is_err());
    }
    #[test]
    fn checksum_requires_exact_filename_and_unique_value() {
        let hash = "a".repeat(64);
        assert_eq!(checksum_for(&format!("{hash} *UsbEject-v0.2.0.exe"), "UsbEject-v0.2.0.exe").unwrap(), hash);
        assert!(checksum_for(&format!("{hash} other.exe"), "UsbEject-v0.2.0.exe").is_err());
        assert!(checksum_for(&format!("{hash} app.exe\n{hash} app.exe"), "app.exe").is_err());
    }
}
