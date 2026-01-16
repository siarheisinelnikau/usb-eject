# UsbEject 0.1.2 — clearer tray icon

Unpublished local preview revision based on local 0.1.1. No public release is claimed.

## Improved

- Enlarged the visible tray artwork from approximately 11 × 12 to 14 × 14 pixels within the standard 16-pixel icon cell.
- Replaced the narrow disk silhouette with a rounded teal tile and a larger white eject mark, matching the application's accent color.
- Embedded individually rasterized 16, 20, 24, 32, 40, 48, 64 and 256-pixel icons. The tray now loads the Windows small-icon metric independently of the panel artwork.
- Refreshed real application screenshots and added a native resource proof on light and dark backgrounds.

## Validation scope

Formatting, Clippy, all 17 fixture tests, native icon dimensions/alpha bounds, six application client captures, and bounded panel/Settings rendering checks passed locally. The proof loads actual HICON resources and draws them through Windows GDI; it does not capture Explorer or certify physical tray interaction. No Computer Use, real ejection, settings changes or update installation was performed. Settings and device logic are unchanged.

Windows x64 portable asset: `UsbEject-v0.1.2.exe`. Quit the older running preview before opening this EXE, so single-instance restoration does not reopen the old build.
