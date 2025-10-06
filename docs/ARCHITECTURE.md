# Architecture

## Boundaries

`model::Storage` separates discovery and the one device-mutation operation. `eject_checked` re-enumerates, requires the complete selected identity to match and rejects blocked devices before calling the adapter. Tests use in-memory storage implementations, never real device discovery or removal.

The Windows adapter enumerates present `GUID_DEVINTERFACE_DISK` interfaces using SetupAPI, queries storage descriptors and disk numbers with metadata-only handles, enumerates volume GUIDs and mount points and joins volume disk extents to physical disk numbers. It maps the Windows directory to its longest matching mounted volume to protect all disks containing that volume. An unknown system mapping fails closed. Only USB, FireWire, SD and MMC buses enter the candidate list, and Windows must report removable media or a hot-plug device. Spanned/multi-disk volumes and discovered disks sharing a direct parent are blocked. Protected system disks still participate in shared-parent counting.

The fingerprint contains PnP instance ID, direct parent instance ID, interface path, current disk number, serial and sorted volume GUIDs. Disk numbers alone are never persistent identities. A `WM_DEVICECHANGE` generation invalidates snapshots and prepared confirmations, including inside a native menu's nested message loop. Before requesting removal, the adapter resolves the PnP instance again, checks its parent and interface metadata, releases its disk handle and checks generation/cancellation again. This is conservative validation, not an atomic transaction with hot-plug; Windows owns the final removal decision.

The sole eject call is `CM_Request_Device_EjectW` on the selected disk devnode. It receives non-null veto buffers to keep Windows' modal UI out of the path. It does not ascend to a hub, lock/dismount volumes or request force flags. Success means Windows accepted this disk's removal. Failure retains the Configuration Manager code, documented veto type and veto source.

## Diagnostics

A refusal triggers a bounded Toolhelp process enumeration with limited query access. Programs whose executable paths lie on the device are displayed as candidates, not proven lock owners. On explicit **Inspect file**, a canonical existing file path must match one of the selected device's mounts. A short Restart Manager session registers that exact file, retrieves application names/PIDs and ends the session. No shutdown or restart API is called. The application does not enumerate every file or system handle.

## UI and lifecycle

The main thread runs `GetMessageW`; it blocks when idle. Standard Win32 controls render the UI. One storage/settings worker and one network worker block on bounded channels. Completion events wake the HWND with `PostMessageW`; network progress is limited to percentage changes. Device events set a one-shot debounced refresh timer, not a periodic polling loop. Storage work is serialized, while a slow update check cannot block storage operations. Worker threads do not hold UI locks.

Closing hides the actual window when the tray icon exists. Tray click, Open and a duplicate launch share restoration behavior: show/restore followed by foreground activation within Windows rules. Explorer's `TaskbarCreated` message reinstalls the tray icon. Quit is handled while hidden, and refuses to exit while a removal request, settings save or update handoff is active. Hidden windows still dispatch completion/device messages. A named session-local mutex prevents duplicate instances. No TCP listener, keyboard simulation or forced focus loop is used.

Settings initialization/save occurs on the storage worker. The startup update command is enqueued only after valid settings load and only when the persistent setting is enabled. A corrupt file is not silently reset or overwritten. User aliases do not affect matching/ejection identity.

## Update protocol

1. An unauthenticated HTTPS request reads the canonical repository's latest published stable release. Semantic versions, release channel, exact asset names, sizes and canonical download URLs are checked.
2. A user click starts a bounded-size download to a fresh app-owned directory. Truncated/oversized downloads, I/O errors and cancellation abort staging. `SHA256SUMS` must contain a unique exact-name entry. The staged file must hash correctly and have a Windows x64 PE header.
3. The staged executable starts itself as a helper with argument arrays, never shell-built commands. It checks its staging path, filename/version, checksum and the parent process's actual executable path. It opens a synchronization handle to that parent and signals readiness before the old UI exits.
4. After parent exit, the helper verifies the old executable has not changed, creates an exclusive backup and incoming copy, verifies the copy and replaces the destination. It restarts the application. A process-creation failure attempts to restore and launch the old executable; a later crash of a successfully created process is not automatically detected. Settings are outside the executable directory and are untouched. An app-owned update receipt is shown on subsequent launches.

This requires a user-writable portable installation folder and compatible helper protocol in future versions. Downloads/checksums rely on GitHub HTTPS and repository control, not an independent signature authority. No check or installation is performed by build scripts or tests. Full update/restart and rollback validation is deferred.

## Primary references

- [Configuration Manager eject and veto semantics](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_request_device_ejectw)
- [Storage device numbers and their lifetime](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_storage_get_device_number)
- [Storage descriptors](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-storage_device_descriptor)
- [SetupAPI interface details](https://learn.microsoft.com/en-us/windows/win32/api/setupapi/nf-setupapi-setupdigetdeviceinterfacedetailw)
- [Device notifications](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerdevicenotificationw)
- [Restart Manager](https://learn.microsoft.com/en-us/windows/win32/rstmgr/about-restart-manager)
- [GitHub release API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)
