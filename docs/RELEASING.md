# Release preparation

No repository, tag or release is published by the current task. The planned canonical repository is `siarheisinelnikau/usb-eject`. Do not present it as available until it exists.

1. Complete and record the applicable [manual checks](MANUAL_TESTS.md). Refresh README's real application-generated panel and settings screenshots when the UI changes; keep fixture captions accurate. Confirm supported Windows versions and device cases from results. Static client screenshots do not prove physical tray behavior or hardware ejection.
2. Update `Cargo.toml`, regenerate `Cargo.lock` if needed and write readable English release notes from verified changes since the preceding relevant release. For the first release, describe initial capabilities and their actual validation limits.
3. Run `./scripts/verify.ps1` and `./scripts/package.ps1`. Inspect the executable's version resources and the matching SHA-256 entry. Both scripts are noninteractive and do not launch the application.
4. Review `dist/UsbEject-vVERSION.exe`, `SHA256SUMS`, `LICENSE`, `THIRD-PARTY-NOTICES.txt` and `build-info.json`. Include licensing notices in distribution. The updater currently supports one x64 portable EXE only; keep the filename free of an architecture suffix while there is only one architecture variant.
5. Only after separate publication authorization, use the connected GitHub plugin for remote repository operations and asset uploads. Do not fall back to remote Git commands, `gh`, shell HTTP, credentials or login flows. If the plugin lacks asset-upload capability, report the exact missing capability.
6. Publish a non-draft, non-prerelease tag such as `v0.1.0`, with human-readable notes and the exact executable/checksum assets. The updater checks the canonical repository, semantic versions, channel, asset URL/name and size. A link-only release cannot satisfy in-app installation.
7. Validate the public update protocol on a disposable writable installation before promising automatic installation. Future releases must retain compatibility with `--apply-update PARENT_PID DESTINATION SHA256`, the ready-file handshake and settings schema 1, or provide a migration path.

The CI configuration builds and stores review artifacts. It has read-only repository permission and no release-publishing step. CI has been prepared locally but has not run on GitHub in this task.
