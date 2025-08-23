//! Read-only enumeration plus a single, explicit Configuration Manager eject call.
use super::{from_wide, wide};
use crate::model::{Device, EjectResult, Identity, Storage, Volume, veto_reason};
use std::{
    mem::{offset_of, size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*,
    Foundation::*,
    Storage::FileSystem::*,
    System::{
        Diagnostics::ToolHelp::*, IO::DeviceIoControl, Ioctl::*, RestartManager::*,
        SystemInformation::GetWindowsDirectoryW, Threading::*,
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn open(path: &str) -> Result<Handle, String> {
    let path = wide(path);
    // Desired access zero: querying metadata does not request write access.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(Handle(handle))
    }
}
fn ioctl(handle: &Handle, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, String> {
    let mut returned = 0;
    let result = unsafe {
        DeviceIoControl(
            handle.0,
            code,
            if input.is_empty() {
                null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else if returned as usize > output.len() {
        Err("Invalid device response length".into())
    } else {
        Ok(returned as usize)
    }
}
// The private call sites only use Win32 plain-data output structures.
fn query<T: Copy>(handle: &Handle, code: u32) -> Result<T, String> {
    let mut output = vec![0u8; size_of::<T>()];
    let count = ioctl(handle, code, &[], &mut output)?;
    if count < size_of::<T>() {
        return Err("Incomplete device metadata".into());
    }
    Ok(unsafe { std::ptr::read_unaligned(output.as_ptr().cast::<T>()) })
}

fn extents(volume: &str) -> Result<Vec<u32>, String> {
    let handle = open(volume.trim_end_matches('\\'))?;
    let mut bytes = vec![0u8; 65536];
    let length = ioctl(
        &handle,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        &mut bytes,
    )?;
    parse_extents(&bytes[..length])
}
fn parse_extents(bytes: &[u8]) -> Result<Vec<u32>, String> {
    if bytes.len() < 4 {
        return Err("Missing volume extents".into());
    }
    let count = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let start = offset_of!(VOLUME_DISK_EXTENTS, Extents);
    if count == 0 || count > 1024 || start + count * size_of::<DISK_EXTENT>() > bytes.len() {
        return Err("Invalid volume extents".into());
    }
    let mut disks: Vec<_> = (0..count)
        .map(|i| {
            let offset = start + i * size_of::<DISK_EXTENT>();
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        })
        .collect();
    disks.sort_unstable();
    disks.dedup();
    Ok(disks)
}

fn enumerate_volumes() -> Result<Vec<Volume>, String> {
    let mut name = [0u16; 1024];
    let finder = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if finder == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Cannot enumerate volumes: {}",
            std::io::Error::last_os_error()
        ));
    }
    struct Finder(HANDLE);
    impl Drop for Finder {
        fn drop(&mut self) {
            unsafe {
                FindVolumeClose(self.0);
            }
        }
    }
    let finder = Finder(finder);
    let mut volumes = Vec::new();
    loop {
        let id = from_wide(&name);
        let mut paths = vec![0u16; 32768];
        let mut required = 0;
        let mut mounts = Vec::new();
        if unsafe {
            GetVolumePathNamesForVolumeNameW(
                name.as_ptr(),
                paths.as_mut_ptr(),
                paths.len() as u32,
                &mut required,
            )
        } != 0
        {
            for part in paths.split(|c| *c == 0).take_while(|s| !s.is_empty()) {
                mounts.push(String::from_utf16_lossy(part));
            }
        }
        let mut label = [0u16; 261];
        unsafe {
            GetVolumeInformationW(
                name.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                0,
            );
        }
        let (mut free, mut total, mut available) = (0, 0, 0);
        let has_space =
            unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut available, &mut total, &mut free) }
                != 0;
        // An unmapped volume is retained so the system-volume guard can fail closed.
        volumes.push(Volume {
            id: id.clone(),
            mounts,
            label: from_wide(&label),
            capacity: has_space.then_some(total),
            free: has_space.then_some(free),
            disk_numbers: extents(&id).unwrap_or_default(),
        });
        if unsafe { FindNextVolumeW(finder.0, name.as_mut_ptr(), name.len() as u32) } == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_NO_MORE_FILES {
                return Err(format!(
                    "Volume discovery was interrupted (Windows error {error})"
                ));
            }
            break;
        }
    }
    Ok(volumes)
}

fn descriptor(handle: &Handle) -> Result<(String, i32), String> {
    // Initialize padding too; never expose uninitialized struct bytes to FFI.
    let mut input = vec![0u8; size_of::<STORAGE_PROPERTY_QUERY>()];
    input[..4].copy_from_slice(&StorageDeviceProperty.to_le_bytes());
    input[4..8].copy_from_slice(&PropertyStandardQuery.to_le_bytes());
    let mut bytes = vec![0u8; 65536];
    let count = ioctl(handle, IOCTL_STORAGE_QUERY_PROPERTY, &input, &mut bytes)?;
    parse_descriptor(&bytes[..count])
}
fn parse_descriptor(bytes: &[u8]) -> Result<(String, i32), String> {
    if bytes.len() < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return Err("Incomplete storage descriptor".into());
    }
    let read_u32 = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let size = read_u32(offset_of!(STORAGE_DEVICE_DESCRIPTOR, Size)) as usize;
    if size < size_of::<STORAGE_DEVICE_DESCRIPTOR>() || size > bytes.len() {
        return Err("Storage descriptor length is invalid".into());
    }
    let serial_offset =
        read_u32(offset_of!(STORAGE_DEVICE_DESCRIPTOR, SerialNumberOffset)) as usize;
    let serial = if serial_offset == 0 {
        String::new()
    } else {
        if serial_offset < size_of::<STORAGE_DEVICE_DESCRIPTOR>() || serial_offset >= size {
            return Err("Storage serial offset is invalid".into());
        }
        let part = &bytes[serial_offset..size];
        let end = part
            .iter()
            .position(|b| *b == 0)
            .ok_or("Unterminated storage serial")?;
        String::from_utf8_lossy(&part[..end]).trim().to_owned()
    };
    Ok((
        serial,
        read_u32(offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType)) as i32,
    ))
}
fn removable_or_hotplug(handle: &Handle) -> bool {
    let mut bytes = [0u8; size_of::<STORAGE_HOTPLUG_INFO>()];
    ioctl(handle, IOCTL_STORAGE_GET_HOTPLUG_INFO, &[], &mut bytes).is_ok_and(|count| {
        count == bytes.len()
            && (bytes[offset_of!(STORAGE_HOTPLUG_INFO, DeviceHotplug)] != 0
                || bytes[offset_of!(STORAGE_HOTPLUG_INFO, MediaRemovable)] != 0)
    })
}

fn property_string(set: HDEVINFO, info: &SP_DEVINFO_DATA, property: u32) -> String {
    let mut buffer = [0u16; 1024];
    if unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            info,
            property,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 2) as u32,
            null_mut(),
        )
    } == 0
    {
        String::new()
    } else {
        from_wide(&buffer)
    }
}

pub struct WindowsStorage {
    pub generation: Arc<AtomicU64>,
    pub expected_generation: u64,
}
impl Storage for WindowsStorage {
    fn discover(&self) -> Result<Vec<Device>, String> {
        let volumes = enumerate_volumes()?;
        let mut windows = [0u16; 32768];
        let len = unsafe { GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) };
        let windows_path = from_wide(&windows).to_lowercase();
        let system = if len == 0 || len as usize >= windows.len() {
            None
        } else {
            volumes
                .iter()
                .filter(|v| {
                    v.mounts
                        .iter()
                        .any(|m| windows_path.starts_with(&m.to_lowercase()))
                })
                .max_by_key(|v| {
                    v.mounts
                        .iter()
                        .filter(|m| windows_path.starts_with(&m.to_lowercase()))
                        .map(String::len)
                        .max()
                        .unwrap_or(0)
                })
        };
        let protected = system.map(|v| v.disk_numbers.clone()).unwrap_or_default();
        let set = unsafe {
            SetupDiGetClassDevsW(
                &GUID_DEVINTERFACE_DISK,
                null(),
                null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        };
        if set == INVALID_HANDLE_VALUE as HDEVINFO {
            return Err(format!(
                "Cannot enumerate storage: {}",
                std::io::Error::last_os_error()
            ));
        }
        struct Set(HDEVINFO);
        impl Drop for Set {
            fn drop(&mut self) {
                unsafe {
                    SetupDiDestroyDeviceInfoList(self.0);
                }
            }
        }
        let set = Set(set);
        let mut devices = Vec::new();
        for index in 0..1024 {
            let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { zeroed() };
            interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if unsafe {
                SetupDiEnumDeviceInterfaces(
                    set.0,
                    null(),
                    &GUID_DEVINTERFACE_DISK,
                    index,
                    &mut interface,
                )
            } == 0
            {
                let error = unsafe { GetLastError() };
                if error == ERROR_NO_MORE_ITEMS {
                    break;
                }
                return Err(format!(
                    "Storage enumeration failed (Windows error {error})"
                ));
            }
            let mut required = 0;
            unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    null_mut(),
                    0,
                    &mut required,
                    null_mut(),
                );
            }
            if required < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 || required > 65536
            {
                continue;
            }
            // u64 storage guarantees alignment of the Win32 detail structure.
            let mut allocation = vec![0u64; (required as usize).div_ceil(8)];
            let detail = allocation
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            let mut info: SP_DEVINFO_DATA = unsafe { zeroed() };
            info.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            unsafe {
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            }
            if unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    detail,
                    required,
                    null_mut(),
                    &mut info,
                )
            } == 0
            {
                continue;
            }
            let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
            let path = from_wide(unsafe {
                std::slice::from_raw_parts(
                    (detail.cast::<u8>().add(offset)).cast::<u16>(),
                    (required as usize - offset) / 2,
                )
            });
            let Ok(handle) = open(&path) else {
                continue;
            };
            let Ok(number) =
                query::<STORAGE_DEVICE_NUMBER>(&handle, IOCTL_STORAGE_GET_DEVICE_NUMBER)
            else {
                continue;
            };
            let Ok((serial, bus)) = descriptor(&handle) else {
                continue;
            };
            let bus_name = if bus == BusTypeUsb {
                "USB"
            } else if bus == BusType1394 {
                "FireWire"
            } else if bus == BusTypeSd {
                "SD"
            } else if bus == BusTypeMmc {
                "MMC"
            } else {
                continue;
            };
            // The Windows disk itself is never exposed in the candidate list.
            if protected.contains(&number.DeviceNumber) {
                continue;
            }
            let hotplug = removable_or_hotplug(&handle);
            let mut id = [0u16; 2048];
            if unsafe {
                SetupDiGetDeviceInstanceIdW(
                    set.0,
                    &info,
                    id.as_mut_ptr(),
                    id.len() as u32,
                    null_mut(),
                )
            } == 0
            {
                continue;
            }
            let instance_id = from_wide(&id);
            let mut name = property_string(set.0, &info, SPDRP_FRIENDLYNAME);
            if name.is_empty() {
                name = property_string(set.0, &info, SPDRP_DEVICEDESC);
            }
            if name.is_empty() {
                name = format!("External disk {}", number.DeviceNumber);
            }
            let mapped: Vec<_> = volumes
                .iter()
                .filter(|v| v.disk_numbers.contains(&number.DeviceNumber))
                .cloned()
                .collect();
            let mut volume_ids: Vec<_> = mapped.iter().map(|v| v.id.clone()).collect();
            volume_ids.sort();
            let blocked = if protected.is_empty() {
                Some("Windows system disk could not be identified; ejection disabled".into())
            } else if !hotplug {
                Some("Windows does not report removable media or a hot-plug device".into())
            } else if mapped.iter().any(|v| v.disk_numbers.len() != 1) {
                Some("Spanned or multi-disk volumes are not supported".into())
            } else {
                None
            };
            let capacity = query::<GET_LENGTH_INFORMATION>(&handle, IOCTL_DISK_GET_LENGTH_INFO)
                .ok()
                .and_then(|n| u64::try_from(n.Length).ok());
            devices.push(Device {
                identity: Identity {
                    instance_id,
                    interface_path: path,
                    disk_number: number.DeviceNumber,
                    serial,
                    volumes: volume_ids,
                },
                name,
                bus: bus_name.into(),
                capacity,
                volumes: mapped,
                blocked,
            });
        }
        devices.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then(a.identity.instance_id.cmp(&b.identity.instance_id))
        });
        devices.dedup_by(|a, b| a.identity.instance_id == b.identity.instance_id);
        Ok(devices)
    }

    fn request_eject(&self, device: &Device, cancel: &AtomicBool) -> Result<EjectResult, String> {
        if self.generation.load(Ordering::Acquire) != self.expected_generation {
            return Ok(EjectResult::Stale);
        }
        let mut devinst = 0;
        let id = wide(&device.identity.instance_id);
        if unsafe { CM_Locate_DevNodeW(&mut devinst, id.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
            != CR_SUCCESS
        {
            return Ok(EjectResult::Stale);
        }
        // Check the interface again immediately before the PnP request; release all disk handles first.
        {
            let handle = open(&device.identity.interface_path)?;
            let number = query::<STORAGE_DEVICE_NUMBER>(&handle, IOCTL_STORAGE_GET_DEVICE_NUMBER)?;
            let (serial, _) = descriptor(&handle)?;
            if number.DeviceNumber != device.identity.disk_number
                || serial != device.identity.serial
            {
                return Ok(EjectResult::Stale);
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(EjectResult::Cancelled);
        }
        if self.generation.load(Ordering::Acquire) != self.expected_generation {
            return Ok(EjectResult::Stale);
        }
        let mut veto = 0;
        let mut veto_name = [0u16; 260];
        // No force, disk-offline, dismount or process-termination APIs are used.
        // Supplying a veto buffer suppresses Windows' own modal message box.
        let result = unsafe {
            CM_Request_Device_EjectW(
                devinst,
                &mut veto,
                veto_name.as_mut_ptr(),
                veto_name.len() as u32,
                0,
            )
        };
        if result == CR_SUCCESS {
            return Ok(EjectResult::Removed);
        }
        let source = from_wide(&veto_name);
        let applications = processes_on_device(device);
        Ok(EjectResult::Refused { reason: format!("{}{} (Configuration Manager 0x{result:08X}, veto {veto})", veto_reason(veto), if source.is_empty() { String::new() } else { format!(": {source}") }), applications,
            diagnostics: "Close files, Explorer windows, terminals and backup jobs on this drive, then retry. Listed programs run from this drive and are candidates, not a complete handle scan. To identify users of a specific file, enter its full path and choose Inspect file. Windows may withhold protected-process information.".into() })
    }
}

fn processes_on_device(device: &Device) -> Vec<String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return vec![];
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut results = Vec::new();
    let mut count = 0;
    while more != 0 && count < 8192 {
        count += 1;
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
        if !process.is_null() {
            let process = Handle(process);
            let mut path = [0u16; 32768];
            let mut size = path.len() as u32;
            if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut size) }
                != 0
            {
                let path = from_wide(&path);
                if path_on_device(&path, device) {
                    results.push(format!(
                        "{} (PID {}, executable on drive)",
                        from_wide(&entry.szExeFile),
                        entry.th32ProcessID
                    ));
                }
            }
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    results.sort();
    results.dedup();
    results
}

pub fn path_on_device(path: &str, device: &Device) -> bool {
    let path = path.strip_prefix("\\\\?\\").unwrap_or(path).to_lowercase();
    device
        .volumes
        .iter()
        .flat_map(|v| &v.mounts)
        .any(|mount| path.starts_with(&mount.to_lowercase()))
}

pub fn inspect_file(device: &Device, path: &str) -> Result<Vec<String>, String> {
    let canonical =
        std::fs::canonicalize(path).map_err(|e| format!("Cannot resolve this file: {e}"))?;
    if !canonical.is_file() || !path_on_device(&canonical.to_string_lossy(), device) {
        return Err("Choose an existing file on one of this device's mounted volumes".into());
    }
    let mut session = 0;
    let mut key = [0u16; 33];
    let result = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if result != 0 {
        return Err(format!("Restart Manager session failed ({result})"));
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }
    let session = Session(session);
    let name = wide(&canonical.to_string_lossy());
    let files = [name.as_ptr()];
    let result = unsafe { RmRegisterResources(session.0, 1, files.as_ptr(), 0, null(), 0, null()) };
    if result != 0 {
        return Err(format!("Cannot register file for diagnostics ({result})"));
    }
    for _ in 0..3 {
        let (mut needed, mut count, mut reboot) = (0, 0, 0);
        let result =
            unsafe { RmGetList(session.0, &mut needed, &mut count, null_mut(), &mut reboot) };
        if result == 0 {
            return Ok(vec![]);
        }
        if result != ERROR_MORE_DATA || needed > 4096 {
            return Err(format!(
                "Cannot query applications using this file ({result})"
            ));
        }
        let mut apps = vec![RM_PROCESS_INFO::default(); needed as usize];
        count = needed;
        let result = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                apps.as_mut_ptr(),
                &mut reboot,
            )
        };
        if result == ERROR_MORE_DATA {
            continue;
        }
        if result != 0 {
            return Err(format!("File diagnostics failed ({result})"));
        }
        return Ok(apps[..count as usize]
            .iter()
            .map(|p| {
                format!(
                    "{} (PID {})",
                    from_wide(&p.strAppName),
                    p.Process.dwProcessId
                )
            })
            .collect());
    }
    Err("Application list changed repeatedly; retry diagnostics".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_extent_buffers_are_rejected() {
        assert!(parse_extents(&[]).is_err());
        assert!(parse_extents(&[255, 255, 255, 255]).is_err());
        let mut buffer =
            vec![0u8; offset_of!(VOLUME_DISK_EXTENTS, Extents) + 2 * size_of::<DISK_EXTENT>()];
        buffer[..4].copy_from_slice(&2u32.to_le_bytes());
        let start = offset_of!(VOLUME_DISK_EXTENTS, Extents);
        buffer[start..start + 4].copy_from_slice(&7u32.to_le_bytes());
        let second = start + size_of::<DISK_EXTENT>();
        buffer[second..second + 4].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(parse_extents(&buffer).unwrap(), [7, 8]);
        assert!(parse_extents(&buffer[..second]).is_err());
    }
}
