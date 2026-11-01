# UsbEject 0.2.0 — language and appearance preferences

This revision adds language and appearance preferences to the compact native drive utility. The source comparison baseline is local v0.1.3; the preceding published-release metadata was not available during this code-only implementation pass.

## Added

- English, Russian, French, German, Spanish and Italian interfaces. Settings offers native language names and applies the saved choice without restarting.
- Saved Light and Dark themes, covering the drive panel, Settings, Drive details, native input backgrounds, owner-drawn actions and window captions. Windows-owned context-menu chrome retains the Windows menu appearance.
- Localized device-removal guidance, protection and veto reasons, tray commands, notifications and update controls. Device names, file paths, serials, Windows codes and diagnostic sources remain unchanged.

## Improved

- Settings has room for the two appearance controls and longer translated update actions. Existing aliases and startup-update preferences survive migration; older settings default to English and Light.
- Theme changes replace retained native font and brush resources without introducing a recurring redraw timer or painting hidden windows. A failed preference save restores the saved selection and reports the failure.
- Catalog checks cover all six languages, interpolation variables, explicit UI lookup coverage and the preservation of device/diagnostic data.

## Validation and limits

Formatting, all-target Clippy with warnings denied, all 22 isolated fixture tests, optimized Windows x64 packaging, PE/version/manifest/checksum inspection and native icon-resource checks passed. See the current validation record for exact commands and evidence. This revision does not launch the GUI, enumerate/eject real drives, install updates, operate the desktop or claim fresh screenshot/runtime-load evidence. Existing README images are explicitly historical v0.1.3 captures; localized and dark-theme screenshots remain pending.

Windows x64 portable asset: `UsbEject-v0.2.0.exe`, accompanied by `SHA256SUMS` and license notices. Quit a running older preview before opening the new EXE, because a second normal launch restores the existing single instance. Settings stay in `%LOCALAPPDATA%\UsbEject\settings.json`; older previews may reject the new appearance fields rather than overwrite them.
