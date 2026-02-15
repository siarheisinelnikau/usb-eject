# UsbEject 0.1.3 — a smaller, steadier tray panel

Unpublished local preview revision based on local 0.1.2. No public release is claimed.

## Fixed

- Native popup menus now run outside the controller's state borrow. The previous More path held that borrow during Windows' nested menu loop, so owner-draw callbacks could repeatedly post redraw requests. Reentrant drawing now marks one pending window instead of posting another redraw message.
- Settings and Drive details are sized and positioned near the tray anchor before showing. They share the panel's work-area clamping instead of opening at Windows' unrelated default location.
- Long operation results have room to remain visible. The Settings status scrollbar appears only when its text overflows.

## Simplified

- Removed Settings, More and Refresh buttons from the panel, together with the repeated section heading, slogan and tray-status footer.
- The empty client is 380 × 180 pixels at 100%, down from 440 × 344. A two-drive panel with a routine message is 380 × 320, down from 440 × 476.
- Right-click the tray icon for Settings, Refresh drives and Quit. Panel context-menu access, Ctrl+, for Settings, F5 for refresh and Ctrl+Q for Quit remain available.

## Validation scope

Formatting, Clippy, 17 existing fixture tests and three placement regressions passed. Fresh real client screenshots, native owner-draw/menu-scope probes and bounded panel/Settings load checks passed. The menu probe exercises the actual production dispatch guard and native callback without opening an OS popup or injecting input. Physical menu/tray clicks, mixed-monitor activation, real ejection and update installation remain manual checks. No Computer Use, user settings or real device operation was used.

Windows x64 portable asset: `UsbEject-v0.1.3.exe`. Quit the running older preview before opening this EXE; otherwise single-instance restoration will reopen the old build.
