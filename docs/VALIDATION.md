# Local validation record

## Tray-panel redesign — 2026-10-31

The current interface uses a compact native tray panel, a separate Settings window and per-drive Details. The primary flow is explicit selection followed by **Safely eject**. No drive is selected automatically after discovery. No real ejection was performed during this work. The final redesign review used no Computer Use.

Completed against the optimized x64 executable:

- Formatting, Clippy with warnings denied, and all **17 fixture tests** passed.
- Six fresh application-generated client PNGs were saved through the same native controls, GDI drawing, fonts and layout as ordinary operation, then inspected as files. Screens cover empty, selected sample drives, refusal, Settings, Details and 150% scale. All capture processes exited with code 0. These are not HTML mockups, desktop photographs or generated artwork. The sample drive identities/capacities and refusal are synthetic; no adapter or user settings/network operation is started in diagnostic mode.
- Visual iteration corrected a client-coordinate offset in the initial capture path, unavailable-field/label issues, clipped status text and unnecessary permanent scrollbars in the main panel. The final path uses `PrintWindow(PW_CLIENTONLY)` and records the actual selected font faces; Segoe UI is used.
- Native CPU GDI and a blocking `GetMessageW` loop were confirmed in source and the packaged imports. Hidden windows defer layout and ignore ordinary hidden paint/print requests; a deliberate finite capture is the only diagnostic exception. The first hidden-state probe exposed an unwanted hidden layout, which was fixed before acceptance.
- Parent `WM_PAINT`, `WM_PRINTCLIENT`, owner-draw and layout counters are separate. Startup and restore produce useful work; the measured settled intervals below contain none. Earlier nonzero Settings counts triggered a diagnostic trace and repeat verification instead of being treated as a pass. Native backdrop/print requests are recorded separately from ordinary paint events.
- Both panel and Settings diagnostic instances completed a bounded visible → hidden-with-data-change → restored cycle. Five seconds of startup settling precede the first clean sample. Each process exited after about 24 seconds; the external script enforces a 30-second deadline. GPU/presentation telemetry was not collected, and Windows desktop composition remains enabled.

| Surface/state | Interval | Added layouts / paints / owner-draw / print-client | CPU increase, one core |
| --- | --- | --- | --- |
| Panel, visible idle | 5.028 s | 0 / 0 / 0 / 0 | Not measurable in this sample |
| Panel, hidden with fixture update | 5.011 s | 0 / 0 / 0 / 0 | Not measurable in this sample |
| Panel, restored idle | 5.030 s | 0 / 0 / 0 / 0 | Not measurable in this sample |
| Settings, visible idle | 5.003 s | 0 / 0 / 0 / 0 | Not measurable in this sample |
| Settings, hidden with fixture update | 5.002 s | 0 / 0 / 0 / 0 | Not measurable in this sample |
| Settings, restored idle | 5.009 s | 0 / 0 / 0 / 0 | Not measurable in this sample |

Raw evidence: [panel load](validation/panel-load.json), [Settings load](validation/settings-load.json), and [capture manifest](screenshots/captures.json). The reports identify the exact executable, PID, backend, available processor count and measurement units. Zero sampled CPU deltas reflect the Windows process-time resolution and short intervals, not a promise of zero CPU on every machine.

The static review and bounded internal hide/restore/rendering checks pass for these fixtures. Physical tray clicks, actual keyboard traversal, foreground activation rules, mixed-monitor DPI behavior, real hardware, slow-driver/network operation and update installation remain unverified. They require later interaction/hardware testing. These limitations do not invalidate the actual saved client screenshots.

## Initial code-only baseline (historical)

Date: 2026-10-31. Scope: unattended, code-only implementation in this repository. No GUI launch, desktop interaction, screenshots, microphone/audio, real device discovery/ejection, updater execution, elevation or remote Git operations were performed. No services, drivers or extensions were installed.

## Completed

- Rust `1.96.1`, Cargo `1.96.1`, Windows MSVC x64 host toolchain. Locked dependencies resolved entirely from the existing local Cargo cache; no dependency downloads were required.
- `cargo fmt --all -- --check`.
- `cargo clippy --locked --offline --all-targets --jobs 2 -- -D warnings`.
- `cargo test --locked --offline --all-targets --jobs 2`: **17 library tests passed**, plus the binary test harness with zero GUI tests. Tests use mocked storage, memory streams, malformed metadata and files only inside the repository's ignored `.test-output` directory. No test calls the live Windows storage/ejection adapter, process diagnostics, networking or update helper.
- `./scripts/package.ps1 -Offline`: release build for `x86_64-pc-windows-msvc`, using two jobs and repository-local output. Prepared `dist/UsbEject-v0.1.0.exe`, checksum, license notices and build metadata.
- `./scripts/inspect-package.ps1`: statically read the packaged PE image. Verified x64 machine, Windows GUI subsystem, embedded version/original filename, icon resource, well-formed manifest, `asInvoker`, `PerMonitorV2`, matching executable/checksum/build metadata and absence of dynamic Visual C++/CRT or external TLS DLL imports. Exact final bytes/hash/imports are recorded in the generated `dist/inspection.json`. This inspection never starts the executable.
- Reviewed the application paths for bounded workers/channels, no idle polling timer, deferred storage/network I/O, system-disk guards, identity checks, a single supported PnP eject call, cooperative cancellation, user-triggered updating, and absence of force-removal/process-kill/elevation code.

Fixture coverage includes cancellation before/during discovery, reused disk numbers with changed serials, changed volume layout, system-device blocking, an unknown system mapping, spanned volumes, a Windows folder mounted at a volume root, grouped-device mutation count, preservation of refusal/application details, actual SDK veto enum values, malformed extent/descriptor buffers, settings round trips and corrupt-file preservation, semantic versions/stable channel, canonical assets and required checksums, duplicate checksums, truncated/oversized/cancelled downloads and I/O failures.

## Environment observations and resolved checks

- The first settings fixture used the OS temporary directory. Sandbox restrictions denied its file-replacement operation. Fixtures were moved into this repository's `.test-output` directory; the same settings replacement test then passed. No escalation was requested.
- `cargo clippy --fix` could not connect to its local parent lock socket (`os error 10013`). Fixes were applied directly and ordinary Clippy subsequently passed. The optional auto-fix mechanism is not required to build or verify this project.
- Incremental development builds sometimes reported that hard links were unavailable and Cargo copied files instead. These are filesystem/toolchain observations, not Rust source lint failures. Release packaging completed successfully.
- Static package inspection found that inline RC manifest strings inserted whitespace before the XML declaration. Build resources now embed a generated manifest file verbatim; final PE manifest parsing passes. No GUI launch was needed to find this packaging issue.

## Not established by these checks

No live device behavior, safe-unplug result, UI appearance/accessibility, tray restore/quit/single-instance behavior, measured idle CPU, Windows version compatibility or end-to-end update/restart/rollback behavior has been verified. Public update metadata was not requested from the planned repository, and no publication availability is claimed. The local CI YAML is prepared but has not run on GitHub.

The EXE remains an unsigned local preview. The redesign section above supersedes the initial absence of application/settings screenshots and isolated rendering measurements. The hardware/interactive checks in [MANUAL_TESTS.md](MANUAL_TESTS.md) still remain before a public-release readiness claim.
