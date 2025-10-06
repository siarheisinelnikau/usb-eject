# Local validation record

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

The packaged EXE is an unsigned local preview for later manual evaluation. Real application/settings screenshots and the hardware/interactive checks in [MANUAL_TESTS.md](MANUAL_TESTS.md) remain required before a public-release readiness claim.
