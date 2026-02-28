//! Product adapter; release discovery, verification and installation live in rust-auto-updator.
pub use rust_auto_updator::{Prepared, Release as Available};
use std::sync::atomic::AtomicBool;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOSITORY: &str = "siarheisinelnikau/usb-eject";

pub fn config(version: &str) -> Result<rust_auto_updator::Config, String> {
    let mut config =
        rust_auto_updator::Config::new("siarheisinelnikau", "usb-eject", version, "UsbEject")?;
    config.checksum_name = "SHA256SUMS".into();
    config.max_executable_size = 100 * 1024 * 1024;
    config.install.stage_prefix = ".usb-eject-update-".into();
    config.install.helper_name = "UsbEject-updater.exe".into();
    Ok(config)
}

pub fn check() -> Result<Option<Available>, String> {
    rust_auto_updator::check_latest(&config(VERSION)?)
}

pub fn download(
    available: &Available,
    cancel: &AtomicBool,
    progress: impl FnMut(u64, u64),
) -> Result<Prepared, String> {
    rust_auto_updator::prepare_download_cancellable(available, &config(VERSION)?, cancel, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(version: &str) -> String {
        let base = format!("https://github.com/{REPOSITORY}/releases");
        let executable = format!("UsbEject-v{version}.exe");
        serde_json::json!({
            "tag_name": format!("v{version}"), "html_url": format!("{base}/tag/v{version}"),
            "draft": false, "prerelease": false, "assets": [
                {"name":executable,"browser_download_url":format!("{base}/download/v{version}/{executable}"),"size":4096},
                {"name":"SHA256SUMS","browser_download_url":format!("{base}/download/v{version}/SHA256SUMS"),"size":100}
            ]
        }).to_string()
    }
    #[test]
    fn product_release_contract_and_saved_checksums_name_are_preserved() {
        let config = config("0.9.0").unwrap();
        assert!(
            rust_auto_updator::parse_release(&release("0.10.0"), &config)
                .unwrap()
                .is_some()
        );
        assert!(
            rust_auto_updator::parse_release(&release("0.9.0"), &config)
                .unwrap()
                .is_none()
        );
        let foreign = release("0.10.0").replace(
            "github.com/siarheisinelnikau/usb-eject/releases/download",
            "example.invalid/download",
        );
        assert!(rust_auto_updator::parse_release(&foreign, &config).is_err());
        let missing = release("0.10.0").replace("SHA256SUMS", "SHA256SUMS.txt");
        assert!(rust_auto_updator::parse_release(&missing, &config).is_err());
    }
}
