//! CLI routing and compatibility with the pre-extraction UsbEject helper protocol.
use crate::{settings, update};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn maybe_apply() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|value| value != "--apply-update") {
        return false;
    }
    let result = (|| {
        let config = update::config(update::VERSION)?;
        if args.len() == 2 {
            return rust_auto_updator::apply_update(&config);
        }
        // Released UsbEject versions invoke the downloaded new executable with
        // PID, destination and SHA. Keep this translation until those hosts retire.
        if args.len() != 5 {
            return Err("Invalid update helper arguments".to_string());
        }
        let pid: u32 = args[2]
            .to_string_lossy()
            .parse()
            .map_err(|_| "Invalid parent process")?;
        let staged = std::env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|e| e.to_string())?;
        let legacy_root = settings::data_dir()?
            .join("updates")
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !staged.starts_with(&legacy_root)
            || staged
                .file_name()
                .is_none_or(|name| name != format!("UsbEject-v{}.exe", update::VERSION).as_str())
        {
            return Err(
                "Legacy update helper is outside the application's staging directory".into(),
            );
        }
        let destination = Path::new(&args[3])
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if destination.starts_with(&legacy_root) {
            return Err("Invalid legacy installation destination".into());
        }
        rust_auto_updator::apply_legacy_update(
            &config,
            &destination,
            &staged,
            &args[4].to_string_lossy(),
            pid,
            Some(&staged.with_extension("ready")),
        )
    })();
    if let Err(error) = result
        && let Ok(root) = settings::data_dir()
    {
        let _ = fs::create_dir_all(&root);
        let _ = fs::write(
            root.join("update-result.txt"),
            format!("Update installation failed: {error}"),
        );
    }
    true
}

/// Cleanup follows successful native UI initialization, never a capture launch.
/// The shared core restricts deletion to the validated application staging plan.
pub fn cleanup_after_start() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|value| value == "--cleanup-update")
        && args.len() == 3
        && let Ok(config) = update::config(update::VERSION)
    {
        rust_auto_updator::cleanup_after_update(PathBuf::from(&args[2]), config.install);
    }
}
