//! Portable self-update helper. Never invoked by tests or build scripts.
use super::from_wide;
use crate::{settings, update};
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::SYNCHRONIZE, System::Threading::*};

pub fn launch(staged: &Path, checksum: &str) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if update::hash_file(staged)? != checksum {
        return Err("Staged update changed before installation".into());
    }
    let mut child = Command::new(staged)
        .arg("--apply-update")
        .arg(std::process::id().to_string())
        .arg(current)
        .arg(checksum)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Cannot start update helper: {e}"))?;
    let ready = staged.with_extension("ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if ready.is_file() {
            return Ok(());
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Update helper could not prepare installation; see update-result.txt in the settings directory".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("Update helper did not become ready; application remains open".into())
}

pub fn maybe_apply() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|v| v != "--apply-update") {
        return false;
    }
    let result = (|| {
        if args.len() != 5 {
            return Err("Invalid update helper arguments".to_string());
        }
        let pid: u32 = args[2]
            .to_string_lossy()
            .parse()
            .map_err(|_| "Invalid parent process")?;
        let destination = Path::new(&args[3]);
        let expected = args[4].to_string_lossy();
        apply(pid, destination, &expected)
    })();
    // An app-owned receipt is shown at the next normal launch. No modal helper UI.
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

fn apply(pid: u32, destination: &Path, expected: &str) -> Result<(), String> {
    let source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let root = settings::data_dir()?
        .join("updates")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !source.starts_with(&root)
        || source
            .file_name()
            .is_none_or(|n| n != format!("UsbEject-v{}.exe", update::VERSION).as_str())
    {
        return Err("Update helper must run from the application's staging directory".into());
    }
    let destination = destination.canonicalize().map_err(|e| e.to_string())?;
    if source == destination || destination.starts_with(&root) {
        return Err("Invalid installation destination".into());
    }
    let parent = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if parent.is_null() {
        return Err("Original application process is unavailable; launch it and retry".into());
    }
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let parent = Process(parent);
    let mut path = [0u16; 32768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(parent.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
        return Err("Cannot verify original application process".into());
    }
    let parent_path = Path::new(&from_wide(&path))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent_path != destination {
        return Err("Update destination does not match the original process".into());
    }
    if expected.len() != 64 || update::hash_file(&source)? != expected {
        return Err("Staged executable failed checksum verification".into());
    }
    update::verify_pe_x64(&source)?;
    let original_hash = update::hash_file(&destination)?;
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(source.with_extension("ready"))
        .map_err(|e| format!("Cannot signal update readiness: {e}"))?;
    if unsafe { WaitForSingleObject(parent.0, 30_000) } != WAIT_OBJECT_0 {
        return Err(
            "Original application did not exit within 30 seconds; update not installed".into(),
        );
    }
    let result = (|| {
        if update::hash_file(&destination)? != original_hash {
            return Err("Installed file changed while waiting; update not installed".into());
        }
        let folder = destination
            .parent()
            .ok_or("Installation folder is missing")?;
        let backup = folder.join(format!("UsbEject.previous-{}.exe", std::process::id()));
        let incoming = folder.join(format!("UsbEject.incoming-{}.exe", std::process::id()));
        // Both files are created exclusively: never overwrite an unrelated file.
        let copy_new = |from: &Path, to: &Path| -> Result<(), String> {
            let mut input = fs::File::open(from).map_err(|e| e.to_string())?;
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(to)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())
        };
        copy_new(&destination, &backup)?;
        copy_new(&source, &incoming)?;
        if update::hash_file(&incoming)? != expected {
            let _ = fs::remove_file(&incoming);
            return Err("Copied update failed verification; original kept".into());
        }
        if let Err(e) = settings::replace(&incoming, &destination) {
            let _ = fs::remove_file(&incoming);
            return Err(format!(
                "Cannot replace the portable executable (move it to a writable folder): {e}"
            ));
        }
        if let Err(e) = Command::new(&destination).spawn() {
            // Restore the original binary if the new executable cannot be started.
            settings::replace(&backup, &destination).map_err(|restore| {
                format!(
                    "Restart failed ({e}); restore failed ({restore}). Backup: {}",
                    backup.display()
                )
            })?;
            return Err(format!("Restart failed; original restored: {e}"));
        }
        Ok(())
    })();
    if let Err(error) = &result {
        if let Ok(root) = settings::data_dir() {
            let _ = fs::write(
                root.join("update-result.txt"),
                format!("Update installation failed: {error}"),
            );
        }
        let _ = Command::new(&destination).spawn();
    }
    result
}
