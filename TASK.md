# UsbEject

Repository: `usb-eject`
Priority: 4 of 15 (user-requested launch order)
Product: UsbEject
Purpose: Clear and reliable external-drive ejection
Inspiration: https://safelyremove.com/

Independent open-source implementation; no affiliation, competitor code or copied assets.

## Requested implementation

Implement a native tray utility listing removable/external storage with useful names, volumes, drive letters and capacity; let users rename display labels and choose a device to eject through supported Windows APIs. Distinguish a physical device from its multiple volumes, avoid exposing internal system disks as ejectable, show progress and clear success/refusal results.

For a busy device, report the Windows veto reason and locking applications when discoverable; help the user resolve the cause, without default force-killing processes or forcing unmounts. Handle unplug/replug, stale device identities and cancellation. Implement the actual Windows discovery/ejection adapter in code with a safe mocked test boundary. No ejection, disk-offline commands, service installation, device changes or real device mutation may be executed in this unattended task.

## Packaging and publication target

Windows standalone asset: `UsbEject-vVERSION.exe` (for the initial release, `UsbEject-v0.1.0.exe`). Append architecture only when distributing multiple executable architectures in the same release.
Planned canonical repository: https://github.com/siarheisinelnikau/usb-eject (not yet created or published; keep availability/error reporting truthful).

## Working agreement

Follow AGENTS.md. Complete as much real implementation and safe noninteractive verification as possible in this unattended code-only session. Manual UI, device and hardware tests will happen later. Record required external components and their verification status. Keep changes committed locally in meaningful increments.

