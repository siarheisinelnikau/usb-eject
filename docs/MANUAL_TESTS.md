# Deferred manual validation

The native client screenshots, bounded idle/hidden diagnostics and fixture tests are recorded in VALIDATION.md. Physical tray clicks, user keyboard traversal, real hardware and update installation below remain pending; Computer Use was explicitly deferred for the redesign. Use disposable drives and synthetic files with backups for later hardware checks. Record Windows version, executable hash, device/bridge model, filesystem and results.

## Native UI and tray

- Inspect main view and settings at 100%, 125%, 150% and 200% DPI, including changing monitors. Check text clipping, selection, long names, keyboard navigation and screen-reader labels.
- Close X, reopen by a single tray click and by Open. Repeat when minimized/hidden; verify foreground behavior and that Quit removes the icon/process.
- Launch twice and verify the same PID/window is restored. Restart Explorer and verify the tray icon returns. Test the documented close behavior when tray creation fails.
- Measure release-build CPU at idle visible and hidden, plus refresh latency and window responsiveness during slow device/network calls. No runtime performance claim has been measured yet.
- Review the application-generated panel and Settings PNGs in README. Capture OS chrome/tray evidence later only if it adds useful coverage beyond these actual client screenshots.

## Device discovery and identity

- Test an empty list, a USB flash drive, USB HDD/SSD, a multi-partition disk, a mounted-folder volume, a locked BitLocker volume and an empty card reader.
- Verify letters, labels, free/capacity values and serial display against Windows. Confirm physical capacity can be unknown without elevation.
- Verify internal SATA/NVMe disks and a USB-hosted Windows system disk cannot be selected for ejection.
- Confirm shared-parent multi-LUN devices and spanned volumes are blocked; confirm no parent hub/controller or sibling device is ejected.
- Test unplug/replug after selection, during discovery, while the tray menu is open and immediately before Safely eject. Reused disk numbers, changing volume layouts and changed parent identities must not authorize the old selection. A newly discovered device must not be selected automatically.
- Test disconnect during a pending Windows request. Confirm final messages describe the Windows result accurately.

## Ejection and diagnostics

- Confirm idle-drive safe removal affects all volumes of the selected disk and does not affect unrelated drives.
- Open synthetic files from several programs, then request removal. Record veto name/type/code and compare with actual Windows refusal. Confirm no process is killed or volume forcibly dismounted.
- For an exact busy file, test Inspect file with multiple owners, no owners, a directory, a missing file, a path outside the device and a junction pointing outside the device. Confirm no complete-volume scan is implied.
- Test protected-process/service/driver refusals and limited-privilege/remote sessions. An access refusal must stay a refusal without requesting elevation.
- Cancel before a request and during discovery. Once inside Configuration Manager, wait for the result; verify the UI explains that it cannot abort the Windows operation.

## Settings and updates

- Rename a display label, restart and verify persistence. Clear it. Verify filesystem volume names are unchanged. Corrupt a copy of the app-owned settings and verify preservation/read-only behavior.
- Disable startup checks and verify zero startup update network requests; manual checking should still work. Re-enable and confirm a background startup request does not delay launch.
- Test unpublished repository/release, 404, offline, timeout, rate limiting, older/equal/newer stable versions, prereleases and incompatible/missing assets. No transient startup modal dialogs should appear.
- Using an explicitly authorized test release, test Download and install: progress, checksum mismatch, incomplete body, cancellation, interrupted download, protected installation folder, helper readiness, process exit wait, restart and executable backup.
- Confirm rollback on launch failure, no replacement after failed staging, label/settings preservation and no unattended download/installation. Inspect persistent failure receipts and manual cleanup of backups/staging after success.
- Confirm packaged execution on a clean Windows 10/11 x64 VM without the development toolchain or VC redistributable installed. Code signing and SmartScreen reputation are not supplied by this preview.
