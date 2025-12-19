# UsbEject 0.1.1 — native painting correction

Unpublished local preview revision based on the local 0.1.0 build. There is no preceding published release or new public release claimed here.

## Fixed

- Native text and controls retain the intended white background when Windows requests their colors during an existing UI state borrow. The previous callback fell back to the system's gray button-face color on that path, although fresh client captures looked correct.
- Layout/font updates queue a coherent parent-and-children redraw after mutation. Owner-draw requests arriving during mutation are deferred instead of falling back to default control painting.
- Notifications in an otherwise empty panel now have room to remain visible, and an active scan shows a loading message.

## Improved

- Removed the duplicate mini-caption from the tray panel and added an accessible Hide to tray button in its header.
- Simplified the empty state by removing the redundant zero count and repeated supporting status. It is shorter and uses lighter button typography.
- Native-checkbox backdrop requests are serviced locally without rebuilding the Settings scene and are identified separately in diagnostics.
- Added a native reentrant palette regression check to every standard visual capture case. Settings and data locations are unchanged.

## Validation scope

Formatting, Clippy, 17 existing fixture tests, real application-generated PNGs and native color-callback regression probes are checked locally. The empty-state palette probe reproduces gray `#F0F0F0` inside the state borrow in 0.1.0 and verifies white `#FFFFFF` inside and outside it after the fix. Populated drives and refusal screenshots use synthetic fixtures. Computer Use, real ejection, update installation and physical tray interaction were not performed for this correction. See VALIDATION.md for the bounded rendering/load evidence and remaining limitations.

Windows x64 portable asset: `UsbEject-v0.1.1.exe`. Quit an already-running older preview before starting this build, so single-instance restoration does not reopen the old version.
