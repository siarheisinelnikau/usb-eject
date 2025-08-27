use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub id: String,
    pub mounts: Vec<String>,
    pub label: String,
    pub capacity: Option<u64>,
    pub free: Option<u64>,
    pub disk_numbers: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub instance_id: String,
    pub parent_instance_id: String,
    pub interface_path: String,
    pub disk_number: u32,
    pub serial: String,
    pub volumes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub identity: Identity,
    pub name: String,
    pub bus: String,
    pub capacity: Option<u64>,
    pub volumes: Vec<Volume>,
    pub blocked: Option<String>,
}

impl Device {
    pub fn display_name<'a>(
        &'a self,
        aliases: &'a std::collections::BTreeMap<String, String>,
    ) -> &'a str {
        aliases
            .get(&self.identity.instance_id)
            .map(String::as_str)
            .unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EjectResult {
    Removed,
    Cancelled,
    Stale,
    Refused {
        reason: String,
        applications: Vec<String>,
        diagnostics: String,
    },
}

/// The only mutation boundary. Mock implementations never call Windows.
pub trait Storage {
    fn discover(&self) -> Result<Vec<Device>, String>;
    fn request_eject(&self, device: &Device, cancel: &AtomicBool) -> Result<EjectResult, String>;
}

pub fn block_reason(
    system_disks: &[u32],
    disk_number: u32,
    hotplug: bool,
    volumes: &[Volume],
) -> Option<String> {
    if system_disks.is_empty() {
        Some("Windows system disk could not be identified; ejection disabled".into())
    } else if system_disks.contains(&disk_number) {
        Some("Windows system disk is protected".into())
    } else if !hotplug {
        Some("Windows does not report removable media or a hot-plug device".into())
    } else if volumes.iter().any(|v| v.disk_numbers.len() != 1) {
        Some("Spanned or multi-disk volumes are not supported".into())
    } else {
        None
    }
}

pub fn system_disks(volumes: &[Volume], windows_directory: &str) -> Vec<u32> {
    let path = format!("{}\\", windows_directory.trim_end_matches('\\')).to_lowercase();
    volumes
        .iter()
        .flat_map(|volume| volume.mounts.iter().map(move |mount| (volume, mount)))
        .filter(|(_, mount)| path.starts_with(&mount.to_lowercase()))
        .max_by_key(|(_, mount)| mount.len())
        .map(|(volume, _)| volume.disk_numbers.clone())
        .unwrap_or_default()
}

/// Re-enumerate before authorizing a mutation; never eject by a saved disk number alone.
pub fn eject_checked(
    storage: &impl Storage,
    selected: &Identity,
    cancel: &AtomicBool,
) -> Result<EjectResult, String> {
    if cancel.load(Ordering::Acquire) {
        return Ok(EjectResult::Cancelled);
    }
    let fresh = storage.discover()?;
    let Some(device) = fresh.iter().find(|d| &d.identity == selected) else {
        return Ok(EjectResult::Stale);
    };
    if let Some(reason) = &device.blocked {
        return Err(format!("Ejection is blocked: {reason}"));
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(EjectResult::Cancelled);
    }
    storage.request_eject(device, cancel)
}

pub fn capacity_text(bytes: Option<u64>) -> String {
    match bytes {
        None => "Unknown".into(),
        Some(n) if n >= 1 << 40 => format!("{:.1} TiB", n as f64 / (1u64 << 40) as f64),
        Some(n) => format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64),
    }
}

pub fn veto_reason(kind: i32) -> &'static str {
    match kind {
        1 => "A legacy device is using this device",
        2 => "A pending close operation is using this device",
        3 => "A Windows application refused removal",
        4 => "A Windows service refused removal",
        5 => "An outstanding handle is open on this device",
        6 => "A device dependency prevents removal",
        7 => "A driver refused removal",
        8 => "The device does not support this removal request",
        9 => "There is insufficient power for this operation",
        10 => "This device cannot be disabled",
        11 => "A legacy driver prevents removal",
        12 => "Windows requires a privilege that is unavailable in this session",
        13 => "The device has already been removed",
        _ => "Windows refused safe removal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    pub fn fixture() -> Device {
        Device {
            identity: Identity {
                instance_id: "USBSTOR\\FIXTURE".into(),
                parent_instance_id: "USB\\FIXTURE".into(),
                interface_path: "fixture-interface".into(),
                disk_number: 5,
                serial: "SAMPLE".into(),
                volumes: vec!["volume-a".into(), "volume-b".into()],
            },
            name: "Test drive".into(),
            bus: "USB".into(),
            capacity: Some(64 << 30),
            volumes: vec![],
            blocked: None,
        }
    }
    struct Mock {
        devices: Vec<Device>,
        requests: Cell<usize>,
        cancel_during_scan: Option<std::sync::Arc<AtomicBool>>,
    }
    impl Storage for Mock {
        fn discover(&self) -> Result<Vec<Device>, String> {
            if let Some(flag) = &self.cancel_during_scan {
                flag.store(true, Ordering::Release);
            }
            Ok(self.devices.clone())
        }
        fn request_eject(&self, _: &Device, _: &AtomicBool) -> Result<EjectResult, String> {
            self.requests.set(self.requests.get() + 1);
            Ok(EjectResult::Removed)
        }
    }
    #[test]
    fn stale_reused_disk_number_never_reaches_mutation() {
        let selected = fixture();
        let mut replacement = fixture();
        replacement.identity.serial = "OTHER".into();
        let mock = Mock {
            devices: vec![replacement],
            requests: Cell::new(0),
            cancel_during_scan: None,
        };
        assert_eq!(
            eject_checked(&mock, &selected.identity, &AtomicBool::new(false)).unwrap(),
            EjectResult::Stale
        );
        assert_eq!(mock.requests.get(), 0);
    }
    #[test]
    fn blocked_system_device_never_reaches_mutation() {
        let mut device = fixture();
        device.blocked = Some("Windows system volume".into());
        let mock = Mock {
            devices: vec![device.clone()],
            requests: Cell::new(0),
            cancel_during_scan: None,
        };
        assert!(eject_checked(&mock, &device.identity, &AtomicBool::new(false)).is_err());
        assert_eq!(mock.requests.get(), 0);
    }
    #[test]
    fn cancellation_during_discovery_prevents_mutation() {
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let device = fixture();
        let mock = Mock {
            devices: vec![device.clone()],
            requests: Cell::new(0),
            cancel_during_scan: Some(flag.clone()),
        };
        assert_eq!(
            eject_checked(&mock, &device.identity, &flag).unwrap(),
            EjectResult::Cancelled
        );
        assert_eq!(mock.requests.get(), 0);
    }
    #[test]
    fn grouped_device_is_requested_exactly_once() {
        let device = fixture();
        let mock = Mock {
            devices: vec![device.clone()],
            requests: Cell::new(0),
            cancel_during_scan: None,
        };
        assert_eq!(
            eject_checked(&mock, &device.identity, &AtomicBool::new(false)).unwrap(),
            EjectResult::Removed
        );
        assert_eq!(mock.requests.get(), 1);
    }
    #[test]
    fn volume_change_invalidates_selection() {
        let old = fixture();
        let mut changed = old.clone();
        changed.identity.volumes.pop();
        let mock = Mock {
            devices: vec![changed],
            requests: Cell::new(0),
            cancel_during_scan: None,
        };
        assert_eq!(
            eject_checked(&mock, &old.identity, &AtomicBool::new(false)).unwrap(),
            EjectResult::Stale
        );
    }

    #[test]
    fn unknown_system_mapping_and_spanned_volumes_fail_closed() {
        assert!(block_reason(&[], 7, true, &[]).is_some());
        assert!(block_reason(&[7], 7, true, &[]).is_some());
        assert!(block_reason(&[0], 7, false, &[]).is_some());
        let volume = Volume {
            id: "spanned".into(),
            mounts: vec!["R:\\".into()],
            label: "Fixture".into(),
            capacity: None,
            free: None,
            disk_numbers: vec![7, 8],
        };
        assert!(block_reason(&[0], 7, true, &[volume]).is_some());
        assert!(block_reason(&[0], 7, true, &[]).is_none());
    }
    #[test]
    fn windows_folder_mount_uses_the_longest_match_even_at_mount_root() {
        let mut root = Volume {
            id: "root".into(),
            mounts: vec!["C:\\".into()],
            label: "".into(),
            capacity: None,
            free: None,
            disk_numbers: vec![0],
        };
        let mounted = Volume {
            id: "windows".into(),
            mounts: vec!["C:\\Windows\\".into()],
            disk_numbers: vec![7],
            ..root.clone()
        };
        assert_eq!(system_disks(&[root.clone(), mounted], "c:\\WINDOWS"), [7]);
        root.disk_numbers.clear();
        assert!(system_disks(&[root], "C:\\Windows").is_empty());
    }
    #[test]
    fn cancelled_request_does_not_even_enumerate() {
        struct NoCalls;
        impl Storage for NoCalls {
            fn discover(&self) -> Result<Vec<Device>, String> {
                panic!("Must not enumerate a cancelled request")
            }
            fn request_eject(&self, _: &Device, _: &AtomicBool) -> Result<EjectResult, String> {
                panic!("Must not eject")
            }
        }
        assert_eq!(
            eject_checked(&NoCalls, &fixture().identity, &AtomicBool::new(true)).unwrap(),
            EjectResult::Cancelled
        );
    }
    #[test]
    fn windows_refusal_and_application_details_are_preserved() {
        struct Refusing;
        impl Storage for Refusing {
            fn discover(&self) -> Result<Vec<Device>, String> {
                Ok(vec![fixture()])
            }
            fn request_eject(&self, _: &Device, _: &AtomicBool) -> Result<EjectResult, String> {
                Ok(EjectResult::Refused {
                    reason: "Outstanding handle".into(),
                    applications: vec!["Fixture editor (PID 123)".into()],
                    diagnostics: "Close the file and retry".into(),
                })
            }
        }
        let result =
            eject_checked(&Refusing, &fixture().identity, &AtomicBool::new(false)).unwrap();
        let EjectResult::Refused {
            reason,
            applications,
            diagnostics,
        } = result
        else {
            panic!("Expected a refusal")
        };
        assert_eq!(reason, "Outstanding handle");
        assert_eq!(applications, ["Fixture editor (PID 123)"]);
        assert!(diagnostics.contains("retry"));
    }
}
