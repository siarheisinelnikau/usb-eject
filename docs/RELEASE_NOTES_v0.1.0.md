# UsbEject 0.1.0 — first local preview

Initial Windows x64 implementation of a native external-drive ejection utility. This is a release-note draft, not an assertion that a public release exists or that hardware validation is complete.

## Added

- A compact native tray panel showing external physical disks, drive letters and available capacities. Settings opens separately; per-drive details expose volumes, local display labels and busy-file diagnostics only when needed.
- Explicit drive selection followed by Safely eject, using Windows Configuration Manager with fresh identity checks, cancellation during preparation and explicit success/refusal results. A scan never automatically selects a new drive.
- System-disk protection and conservative rejection of unknown hot-plug metadata, multi-disk volumes and discovered shared-parent enclosures.
- Windows veto diagnostics, partial process candidates and exact-file Restart Manager inspection without forced shutdown or unmount.
- Persistent settings, installed version, optional startup checks, manual update checking and a user-triggered checksum-verified portable update/restart flow.
- Build/package scripts, locked dependencies, original application artwork, Windows resources, MIT license and generated dependency notices.

## Validation and limitations

Offline compilation, formatting, Clippy and isolated fixture tests have been run locally. Real application-generated client PNGs were reviewed at 100% and 150% scale without Computer Use, using isolated empty/populated/error/settings/detail states. Bounded internal rendering diagnostics cover visible idle, hidden data changes and restored idle. Physical tray clicks, keyboard traversal, real-device discovery/ejection, hardware unplug/replug cases and end-to-end update installation remain pending before a public release. Intended platform: Windows 10 1809+ and Windows 11 x64; single executable asset `UsbEject-v0.1.0.exe`.

Physical capacity can be unavailable without query access. Diagnostics are not an exhaustive handle scan. No force-removal, power-off, parent-hub ejection or unattended installation is offered. The canonical repository/release remains unavailable until separately authorized publication.
