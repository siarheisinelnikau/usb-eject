//! Event-driven Win32 UI. All storage, settings and network I/O runs on bounded workers.
use crate::ui_capture as capture;
use crate::ui_style::{self as style, ACCENT, INK, LINE, MUTED, SURFACE, TINT, Theme, WHITE};
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};
use usb_eject::{
    model::{self, Device, EjectResult, Identity, Storage},
    settings::{self, Settings},
    update::{self, Available},
    windows::{
        from_wide, installer,
        storage::{WindowsStorage, inspect_file},
        wide,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_LEFT, Threading::*},
    UI::{
        Controls::*,
        HiDpi::*,
        Input::KeyboardAndMouse::{EnableWindow, GetKeyState, SetFocus},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const WAKE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const RESTORE: u32 = WM_APP + 3;
const REDRAW: u32 = WM_APP + 4;
const RESCAN_TIMER: usize = 1;
const CLASS: &str = "UsbEject.NativeWindow.v1";
const LIST: u16 = 101;
const REFRESH: u16 = 102;
const EJECT: u16 = 103;
const CANCEL: u16 = 104;
const ALIAS: u16 = 105;
const SAVE_ALIAS: u16 = 106;
const FILE_PATH: u16 = 107;
const INSPECT: u16 = 108;
const STARTUP: u16 = 109;
const CHECK: u16 = 110;
const INSTALL: u16 = 111;
const QUIT: u16 = 112;
const OPEN: u16 = 113;
const SETTINGS: u16 = 114;
const DETAILS: u16 = 115;
const MORE: u16 = 116;
const HIDE: u16 = 117;
const CLOSE_SETTINGS: u16 = 118;
const CLOSE_DETAILS: u16 = 119;
const CANCEL_MAIN: u16 = 120;

enum DiskCommand {
    Initialize,
    Scan(u64),
    Eject(Identity, u64),
    Inspect(Device, String),
    Save(Settings),
}
enum NetCommand {
    Check,
    Install(Available),
}
enum Event {
    Initialized(Result<Settings, String>, Option<String>),
    Scanned(u64, Result<Vec<Device>, String>),
    Ejected(Result<EjectResult, String>),
    Inspected(Result<Vec<String>, String>),
    Saved(Settings, Result<(), String>),
    Checked(Result<Option<Available>, String>),
    Progress(u64, u64),
    Handoff,
    Installed(Result<(), String>),
}

fn send_event(tx: &SyncSender<Event>, hwnd: usize, event: Event) {
    if tx.send(event).is_ok() {
        unsafe {
            PostMessageW(hwnd as HWND, WAKE, 0, 0);
        }
    }
}
fn workers(
    hwnd: HWND,
    generation: Arc<AtomicU64>,
    disk_cancel: Arc<AtomicBool>,
    net_cancel: Arc<AtomicBool>,
) -> (
    SyncSender<DiskCommand>,
    SyncSender<NetCommand>,
    Receiver<Event>,
) {
    let (events, receiver) = mpsc::sync_channel(128);
    let (disk, jobs) = mpsc::sync_channel(4);
    let (net, network) = mpsc::sync_channel(1);
    let hwnd = hwnd as usize;
    let disk_events = events.clone();
    std::thread::spawn(move || {
        // Keep empty card readers and unready media from opening critical-error UI.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(
                windows_sys::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS,
                null_mut(),
            );
        }
        while let Ok(job) = jobs.recv() {
            let event = match job {
                DiskCommand::Initialize => {
                    let root = settings::data_dir();
                    let loaded = root
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|p| Settings::load(&p.join("settings.json")));
                    let receipt = root
                        .ok()
                        .and_then(|p| std::fs::read_to_string(p.join("update-result.txt")).ok());
                    Event::Initialized(loaded, receipt)
                }
                DiskCommand::Scan(epoch) => {
                    let adapter = WindowsStorage {
                        generation: generation.clone(),
                        expected_generation: epoch,
                    };
                    Event::Scanned(epoch, adapter.discover())
                }
                DiskCommand::Eject(identity, epoch) => {
                    let adapter = WindowsStorage {
                        generation: generation.clone(),
                        expected_generation: epoch,
                    };
                    Event::Ejected(model::eject_checked(&adapter, &identity, &disk_cancel))
                }
                DiskCommand::Inspect(device, path) => {
                    Event::Inspected(inspect_file(&device, &path))
                }
                DiskCommand::Save(value) => {
                    let result =
                        settings::data_dir().and_then(|p| value.save(&p.join("settings.json")));
                    Event::Saved(value, result)
                }
            };
            send_event(&disk_events, hwnd, event);
        }
    });
    std::thread::spawn(move || {
        while let Ok(job) = network.recv() {
            let event = match job {
                NetCommand::Check => Event::Checked(update::check()),
                NetCommand::Install(available) => {
                    let result = settings::data_dir()
                        .and_then(|p| {
                            update::download(
                                &available,
                                &p.join("updates"),
                                &net_cancel,
                                |done, total| {
                                    send_event(&events, hwnd, Event::Progress(done, total))
                                },
                            )
                        })
                        .and_then(|(path, checksum)| {
                            if net_cancel.load(Ordering::Acquire) {
                                return Err("Update cancelled before installation".into());
                            }
                            send_event(&events, hwnd, Event::Handoff);
                            installer::launch(&path, &checksum)
                        });
                    Event::Installed(result)
                }
            };
            send_event(&events, hwnd, event);
        }
    });
    (disk, net, receiver)
}

struct App {
    hwnd: HWND,
    settings_hwnd: HWND,
    details_hwnd: HWND,
    controls: Vec<(u16, HWND)>,
    themes: [Theme; 3],
    icon: HICON,
    tray_icon: HICON,
    tray_ok: bool,
    taskbar_created: u32,
    notification: HDEVNOTIFY,
    disk: SyncSender<DiskCommand>,
    net: SyncSender<NetCommand>,
    events: Receiver<Event>,
    generation: Arc<AtomicU64>,
    disk_cancel: Arc<AtomicBool>,
    net_cancel: Arc<AtomicBool>,
    devices: Vec<Device>,
    snapshot_generation: u64,
    settings: Settings,
    settings_ready: bool,
    saving: bool,
    disk_busy: bool,
    ejecting: bool,
    refresh_pending: bool,
    net_busy: bool,
    installing: bool,
    handoff: bool,
    available: Option<Available>,
    menu_open: bool,
    diagnostic: Option<capture::Options>,
    metrics: capture::Metrics,
    dirty: [bool; 3],
    diagnostic_step: u32,
    diagnostic_samples: Vec<capture::Sample>,
    diagnostic_failed: bool,
    paint_events: Vec<capture::PaintEvent>,
    draw_events: Vec<capture::DrawEvent>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DEVICE_EPOCH: Arc<AtomicU64> = Arc::new(AtomicU64::new(1));
    static NATIVE_BACKDROPS: Cell<u64> = const { Cell::new(0) };
}

impl App {
    fn control(&self, id: u16) -> HWND {
        self.controls
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, h)| *h)
            .unwrap_or(null_mut())
    }
    fn text(&self, id: u16, value: &str) {
        let text = wide(value);
        unsafe {
            SetWindowTextW(self.control(id), text.as_ptr());
        }
    }
    fn input(&self, id: u16) -> String {
        let mut value = vec![0u16; 32768];
        unsafe {
            GetWindowTextW(self.control(id), value.as_mut_ptr(), value.len() as i32);
        }
        from_wide(&value)
    }
    fn enabled(&self, id: u16, enabled: bool) {
        unsafe {
            EnableWindow(self.control(id), enabled as i32);
        }
    }
    fn status(&mut self, text: &str) {
        let summary = if text.chars().count() <= 150 {
            text.to_owned()
        } else {
            let first = text
                .split(". ")
                .next()
                .unwrap_or(text)
                .trim_end_matches('.');
            let target = if self.selected().is_some() {
                "Open Details for the full result."
            } else {
                "Open Settings for details."
            };
            if first.chars().count() <= 110 {
                format!("{first}. {target}")
            } else {
                format!("This operation needs attention. {target}")
            }
        };
        self.text(202, &summary);
        self.text(215, text);
        if self.devices.is_empty() {
            self.resize_panel();
            self.layout(self.hwnd);
        }
    }
    fn selected(&self) -> Option<&Device> {
        let index = unsafe { SendMessageW(self.control(LIST), LB_GETCURSEL, 0, 0) };
        if index < 0 {
            None
        } else {
            self.devices.get(index as usize)
        }
    }
    fn update_buttons(&self) {
        let current = self.snapshot_generation == self.generation.load(Ordering::Acquire);
        self.enabled(
            EJECT,
            !self.disk_busy
                && !self.installing
                && current
                && self.selected().is_some_and(|d| d.blocked.is_none()),
        );
        self.enabled(REFRESH, !self.disk_busy && !self.installing);
        self.enabled(
            INSPECT,
            !self.disk_busy && !self.installing && current && self.selected().is_some(),
        );
        self.enabled(CANCEL, (self.disk_busy || self.installing) && !self.handoff);
        self.enabled(
            SAVE_ALIAS,
            self.settings_ready && !self.saving && !self.installing && self.selected().is_some(),
        );
        self.enabled(
            ALIAS,
            self.settings_ready && !self.saving && !self.installing && self.selected().is_some(),
        );
        self.enabled(
            FILE_PATH,
            !self.disk_busy && !self.installing && current && self.selected().is_some(),
        );
        self.enabled(
            STARTUP,
            self.settings_ready && !self.saving && !self.installing,
        );
        self.enabled(CHECK, !self.net_busy);
        self.enabled(DETAILS, self.selected().is_some());
        self.enabled(
            INSTALL,
            self.available.is_some() && !self.net_busy && !self.disk_busy && !self.saving,
        );
        unsafe {
            ShowWindow(
                self.control(INSTALL),
                if self.available.is_some() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            ShowWindow(
                self.control(CANCEL),
                if self.installing { SW_SHOW } else { SW_HIDE },
            );
            ShowWindow(
                self.control(CANCEL_MAIN),
                if self.ejecting { SW_SHOW } else { SW_HIDE },
            );
            ShowWindow(
                self.control(DETAILS),
                if self.ejecting || self.devices.is_empty() {
                    SW_HIDE
                } else {
                    SW_SHOW
                },
            );
        }
    }
    fn selected_changed(&mut self) {
        self.text(FILE_PATH, "");
        if let Some(device) = self.selected() {
            self.text(
                ALIAS,
                self.settings
                    .aliases
                    .get(&device.identity.instance_id)
                    .map(String::as_str)
                    .unwrap_or(""),
            );
            let mut text = format!(
                "{}\r\n{} · physical disk {}\r\nPhysical capacity: {}\r\nSerial: {}\r\n\r\n",
                device.name,
                device.bus,
                device.identity.disk_number,
                model::capacity_text(device.capacity),
                if device.identity.serial.is_empty() {
                    "Not provided"
                } else {
                    &device.identity.serial
                }
            );
            for volume in &device.volumes {
                text.push_str(&format!(
                    "{}  {}\r\nCapacity: {} · Free: {}\r\n{}\r\n\r\n",
                    if volume.mounts.is_empty() {
                        "No mount point".into()
                    } else {
                        volume.mounts.join(", ")
                    },
                    if volume.label.is_empty() {
                        "Unlabeled volume"
                    } else {
                        &volume.label
                    },
                    model::capacity_text(volume.capacity),
                    model::capacity_text(volume.free),
                    volume.id
                ));
            }
            if device.volumes.is_empty() {
                text.push_str("No mapped volumes (media may be absent or locked).\r\n\r\n");
            }
            text.push_str(&format!(
                "Device ID: {}\r\n\r\n{}",
                device.identity.instance_id,
                device.blocked.as_deref().unwrap_or(
                    "Ejection affects this physical disk and all its volumes. Save your work first."
                )
            ));
            self.text(201, &text);
        } else {
            self.text(ALIAS, "");
            self.text(201, "Select an external device to inspect its volumes.\r\n\r\nInternal storage and the Windows system disk are excluded.\r\n\r\nNo external devices found? Connect a USB drive and choose Refresh. Unknown and unsupported bus types are omitted.");
        }
        self.update_buttons();
    }
    fn populate(&mut self, devices: Vec<Device>) {
        let previous = self.selected().map(|d| d.identity.clone());
        self.devices = devices;
        unsafe {
            SendMessageW(self.control(LIST), LB_RESETCONTENT, 0, 0);
        }
        for device in &self.devices {
            let mounts: Vec<_> = device
                .volumes
                .iter()
                .flat_map(|v| v.mounts.iter().cloned())
                .collect();
            let row = wide(&format!(
                "{}{}{}",
                device.display_name(&self.settings.aliases),
                if mounts.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", mounts.join(", "))
                },
                if device.blocked.is_some() {
                    "  (protected)"
                } else {
                    ""
                }
            ));
            unsafe {
                SendMessageW(self.control(LIST), LB_ADDSTRING, 0, row.as_ptr() as isize);
            }
        }
        if let Some(index) =
            previous.and_then(|id| self.devices.iter().position(|d| d.identity == id))
        {
            unsafe {
                SendMessageW(self.control(LIST), LB_SETCURSEL, index, 0);
            }
        }
        self.selected_changed();
        self.resize_panel();
        self.layout(self.hwnd);
    }
    fn scan(&mut self) {
        if self.disk_busy {
            self.refresh_pending = true;
            return;
        }
        let epoch = self.generation.load(Ordering::Acquire);
        if self.disk.try_send(DiskCommand::Scan(epoch)).is_ok() {
            self.disk_busy = true;
            self.refresh_pending = false;
            self.disk_cancel.store(false, Ordering::Release);
            self.text(203, "Scanning…");
            self.text(213, "Looking for drives…");
            self.text(214, "Windows is checking connected storage.");
            self.update_buttons();
        } else {
            self.status("Storage worker is unavailable. Restart UsbEject.");
        }
    }
    fn check_updates(&mut self) {
        if !self.net_busy && self.net.try_send(NetCommand::Check).is_ok() {
            self.net_busy = true;
            self.text(204, "Checking the canonical GitHub repository…");
            self.update_buttons();
        }
    }
    fn save(&mut self, value: Settings) {
        if self.settings_ready
            && !self.saving
            && !self.installing
            && self.disk.try_send(DiskCommand::Save(value)).is_ok()
        {
            self.saving = true;
            self.update_buttons();
        }
    }
    fn command(&mut self, id: u16, notification: u16) {
        if self.diagnostic.is_some()
            && matches!(
                id,
                EJECT | INSPECT | SAVE_ALIAS | STARTUP | CHECK | INSTALL | REFRESH
            )
        {
            return;
        }
        match id {
            LIST if notification == LBN_SELCHANGE as u16 => self.selected_changed(),
            REFRESH => self.scan(),
            EJECT if !self.disk_busy && !self.installing => {
                let Some(device) = self.selected().cloned() else {
                    return;
                };
                let epoch = self.generation.load(Ordering::Acquire);
                if device.blocked.is_some() || epoch != self.snapshot_generation {
                    self.status("The device list changed. Refresh and select the device again.");
                    return;
                }
                if self
                    .disk
                    .try_send(DiskCommand::Eject(device.identity, epoch))
                    .is_ok()
                {
                    self.disk_cancel.store(false, Ordering::Release);
                    self.disk_busy = true;
                    self.ejecting = true;
                    self.status("Validating device identity and requesting safe removal… Cancel can stop preparation; an active Windows PnP request cannot be undone.");
                }
            }
            CANCEL | CANCEL_MAIN if !self.handoff => {
                self.disk_cancel.store(true, Ordering::Release);
                self.net_cancel.store(true, Ordering::Release);
                self.status("Cancellation requested. An active Windows ejection call cannot be interrupted; wait for its result. Downloads stop at the next read boundary.");
            }
            SAVE_ALIAS => {
                if let Some(device) = self.selected() {
                    let mut value = self.settings.clone();
                    let label = self.input(ALIAS).trim().to_owned();
                    if label.chars().count() > 80 || label.chars().any(char::is_control) {
                        self.status("Use a display label of at most 80 characters without control characters.");
                        return;
                    }
                    if label.is_empty() {
                        value.aliases.remove(&device.identity.instance_id);
                    } else {
                        value
                            .aliases
                            .insert(device.identity.instance_id.clone(), label);
                    }
                    self.save(value);
                }
            }
            INSPECT if !self.disk_busy && !self.installing => {
                if let Some(device) = self.selected().cloned() {
                    let path = self.input(FILE_PATH);
                    if self
                        .disk
                        .try_send(DiskCommand::Inspect(device, path))
                        .is_ok()
                    {
                        self.disk_busy = true;
                        self.status(
                            "Asking Restart Manager which applications use this exact file…",
                        );
                    }
                }
            }
            STARTUP => {
                let checked = unsafe { SendMessageW(self.control(STARTUP), BM_GETCHECK, 0, 0) }
                    == BST_CHECKED as isize;
                let mut value = self.settings.clone();
                value.check_updates_on_startup = checked;
                self.save(value);
            }
            CHECK => self.check_updates(),
            INSTALL if !self.net_busy && !self.disk_busy && !self.saving => {
                if let Some(available) = self.available.clone() {
                    self.net_cancel.store(false, Ordering::Release);
                    if self.net.try_send(NetCommand::Install(available)).is_ok() {
                        self.net_busy = true;
                        self.installing = true;
                        self.text(204, "Downloading verified update. UsbEject will close and restart after installation. Settings and display labels are preserved.");
                    }
                }
            }
            QUIT => self.quit(),
            OPEN => self.restore(),
            SETTINGS => self.open_secondary(self.settings_hwnd),
            DETAILS if self.selected().is_some() => self.open_secondary(self.details_hwnd),
            MORE => self.tray_menu(false),
            HIDE => self.hide_panel(),
            CLOSE_SETTINGS => unsafe {
                ShowWindow(self.settings_hwnd, SW_HIDE);
            },
            CLOSE_DETAILS => unsafe {
                ShowWindow(self.details_hwnd, SW_HIDE);
            },
            _ => {}
        }
        self.update_buttons();
    }
    fn process_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Initialized(result, receipt) => {
                    match result {
                        Ok(settings) => {
                            self.settings = settings;
                            self.settings_ready = true;
                            if self.settings.check_updates_on_startup {
                                self.check_updates();
                            } else {
                                self.text(204, "Automatic startup checks are disabled. Manual checking remains available.");
                            }
                        }
                        Err(error) => {
                            self.settings.check_updates_on_startup = false;
                            self.status(&format!("{error}. Settings editing and startup update checks are disabled for this session."));
                            self.text(204, "Startup checks are disabled because settings could not be loaded. Manual checking remains available.");
                        }
                    }
                    unsafe {
                        SendMessageW(
                            self.control(STARTUP),
                            BM_SETCHECK,
                            if self.settings.check_updates_on_startup {
                                BST_CHECKED
                            } else {
                                BST_UNCHECKED
                            } as usize,
                            0,
                        );
                    }
                    if let Some(receipt) = receipt {
                        self.text(204, &receipt);
                    }
                    self.scan();
                }
                Event::Scanned(epoch, result) => {
                    self.disk_busy = false;
                    self.text(213, "No external drives");
                    self.text(214, "Connect a USB drive to see it here.");
                    if epoch == self.generation.load(Ordering::Acquire) {
                        self.snapshot_generation = epoch;
                        match result {
                            Ok(devices) => {
                                self.text(203, &format!("{} drives", devices.len()));
                                self.populate(devices);
                            }
                            Err(error) => {
                                self.populate(vec![]);
                                self.text(203, "Discovery failed");
                                self.status(&error);
                            }
                        }
                    } else {
                        self.refresh_pending = true;
                    }
                }
                Event::Ejected(result) => {
                    self.disk_busy = false;
                    self.ejecting = false;
                    match result {
                        Ok(EjectResult::Removed) => { self.status("Windows accepted safe removal. You can unplug this device. The list will refresh."); self.balloon("Safe removal accepted", "Windows accepted removal of the selected device."); }
                        Ok(EjectResult::Cancelled) => self.status("Ejection cancelled before the Windows request."),
                        Ok(EjectResult::Stale) => self.status("The device changed or was disconnected. No ejection was requested. Select it again after refresh."),
                        Ok(EjectResult::Refused { reason, applications, diagnostics }) => self.status(&format!("Removal refused: {reason}\r\n{}\r\n{diagnostics}", if applications.is_empty() { "No candidate applications discovered. This does not prove the drive is unused.".into() } else { applications.join("; ") })),
                        Err(error) => self.status(&format!("Ejection failed: {error}")),
                    }
                    self.refresh_pending = true;
                }
                Event::Inspected(result) => {
                    self.disk_busy = false;
                    match result {
                        Ok(apps) if apps.is_empty() => self.status("Restart Manager reported no applications for this file. Other files, Explorer, drivers or protected processes may still block ejection."),
                        Ok(apps) => self.status(&format!("Applications using the selected file:\r\n{}\r\nSave your work and close these applications yourself, then retry ejection.", apps.join("\r\n"))),
                        Err(error) => self.status(&error),
                    }
                }
                Event::Saved(value, result) => {
                    self.saving = false;
                    match result {
                        Ok(()) => {
                            let aliases_changed = self.settings.aliases != value.aliases;
                            self.settings = value;
                            if aliases_changed {
                                self.populate(self.devices.clone());
                                self.status(
                                    "Display label saved. Filesystem volume labels are unchanged.",
                                );
                            } else if self.settings.check_updates_on_startup {
                                self.status("Startup update checks enabled. The setting applies at the next launch.");
                            } else {
                                self.status("Startup update checks disabled. Manual checking remains available.");
                            }
                        }
                        Err(error) => self.status(&error),
                    }
                    unsafe {
                        SendMessageW(
                            self.control(STARTUP),
                            BM_SETCHECK,
                            if self.settings.check_updates_on_startup {
                                BST_CHECKED
                            } else {
                                BST_UNCHECKED
                            } as usize,
                            0,
                        );
                    }
                }
                Event::Checked(result) => {
                    self.net_busy = false;
                    self.available = None;
                    match result {
                        Ok(Some(available)) => {
                            self.text(204, &format!("Version {} is available. Download and install closes and restarts the application.", available.version));
                            self.balloon(
                                "UsbEject update available",
                                &format!(
                                    "Version {} is available in Settings / Updates.",
                                    available.version
                                ),
                            );
                            self.available = Some(available);
                        }
                        Ok(None) => self.text(
                            204,
                            "This installation is up to date on the stable release channel.",
                        ),
                        Err(error) => self.text(204, &error),
                    }
                }
                Event::Progress(done, total) => self.text(
                    204,
                    &format!(
                        "Downloading update: {}% ({} / {} KiB). Verifying before installation…",
                        done * 100 / total,
                        done / 1024,
                        total / 1024
                    ),
                ),
                Event::Handoff => {
                    self.handoff = true;
                    self.text(204, "Verification complete. Preparing executable replacement and restart; cancellation is no longer available.");
                }
                Event::Installed(result) => {
                    self.net_busy = false;
                    self.installing = false;
                    self.handoff = false;
                    match result {
                        Ok(()) => {
                            unsafe {
                                DestroyWindow(self.hwnd);
                            }
                            return;
                        }
                        Err(error) => self.text(204, &format!("Update was not installed: {error}")),
                    }
                }
            }
        }
        if self.refresh_pending && !self.disk_busy {
            self.scan();
        }
        self.update_buttons();
    }

    fn prepare_diagnostic(&mut self) {
        let options = self.diagnostic.as_ref().unwrap();
        let screen = options.screen.clone();
        self.settings_ready = true;
        self.snapshot_generation = self.generation.load(Ordering::Acquire);
        self.settings.check_updates_on_startup = screen != "settings";
        unsafe {
            SendMessageW(
                self.control(STARTUP),
                BM_SETCHECK,
                if self.settings.check_updates_on_startup {
                    BST_CHECKED
                } else {
                    BST_UNCHECKED
                } as usize,
                0,
            );
        }
        if matches!(screen.as_str(), "sample" | "error" | "details") || options.measure {
            self.populate(capture::sample_devices());
            unsafe {
                SendMessageW(self.control(LIST), LB_SETCURSEL, 0, 0);
            }
            self.selected_changed();
        } else {
            self.populate(vec![]);
        }
        self.text(203, &format!("{} drives", self.devices.len()));
        self.text(204,"Automatic startup checks are disabled. You can check for updates manually at any time.");
        if screen == "error" {
            self.status("Windows could not remove this drive. Close files and apps using it, then try again. Open Details to inspect a busy file.");
        } else if self.devices.is_empty() {
            self.status("");
        } else {
            self.status("Save open files before ejecting this drive and all its volumes.");
        }
        self.update_buttons();
    }
    fn diagnostic_tick(&mut self) {
        self.diagnostic_step += 1;
        let step = self.diagnostic_step;
        let target = if self
            .diagnostic
            .as_ref()
            .is_some_and(|d| d.screen == "settings")
        {
            self.settings_hwnd
        } else {
            self.hwnd
        };
        if matches!(step, 5 | 10 | 12 | 17 | 19 | 24) {
            self.metrics.native_backdrops = NATIVE_BACKDROPS.with(Cell::get);
            self.diagnostic_samples
                .push(capture::Sample::new(step, self.metrics));
        }
        if step == 10 {
            unsafe {
                ShowWindow(target, SW_HIDE);
            }
        }
        if step == 13 {
            self.populate(capture::sample_devices());
        }
        if step == 17 {
            capture::position_measurement(target);
            unsafe {
                ShowWindow(target, SW_SHOWNOACTIVATE);
            }
            self.layout(target);
        }
        if step >= 24 {
            unsafe {
                KillTimer(self.hwnd, capture::TIMER);
            }
            if let Some(options) = &self.diagnostic
                && let Err(error) = capture::write_measurement(
                    &options.output,
                    &self.diagnostic_samples,
                    &self.paint_events,
                    &self.draw_events,
                )
            {
                eprintln!("{error}");
                self.diagnostic_failed = true;
            }
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
    fn theme_index(&self, hwnd: HWND) -> usize {
        if hwnd == self.settings_hwnd {
            1
        } else if hwnd == self.details_hwnd {
            2
        } else {
            0
        }
    }
    fn can_paint(&self, hwnd: HWND) -> bool {
        (unsafe { IsWindowVisible(hwnd) != 0 })
            || self.diagnostic.as_ref().is_some_and(|d| !d.measure)
    }
    fn window_dpi(&self, hwnd: HWND) -> u32 {
        self.diagnostic
            .as_ref()
            .map(|d| d.scale * 96 / 100)
            .unwrap_or_else(|| unsafe { GetDpiForWindow(hwnd).max(96) })
    }
    fn hide_panel(&mut self) {
        if self.tray_ok || self.diagnostic.is_some() {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        } else {
            self.quit();
        }
    }
    fn restore(&mut self) {
        self.resize_panel();
        self.position_panel();
        unsafe {
            ShowWindow(self.hwnd, SW_SHOWNORMAL);
        }
        self.layout(self.hwnd);
        unsafe {
            SetForegroundWindow(self.hwnd);
            SetFocus(self.control(if self.devices.is_empty() {
                REFRESH
            } else {
                LIST
            }));
        }
    }
    fn open_secondary(&mut self, hwnd: HWND) {
        unsafe {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.layout(hwnd);
        unsafe {
            SetForegroundWindow(hwnd);
            SetFocus(self.control(if hwnd == self.settings_hwnd {
                STARTUP
            } else {
                ALIAS
            }));
        }
    }
    fn quit(&mut self) {
        if self.ejecting || self.installing || self.saving {
            self.status(
                "Wait for the active operation to finish, or cancel preparation before quitting.",
            );
            self.restore();
            return;
        }
        self.disk_cancel.store(true, Ordering::Release);
        self.net_cancel.store(true, Ordering::Release);
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = self.tray_icon;
        copy_wide(&mut data.szTip, "UsbEject · click to safely remove a drive");
        data
    }
    fn add_tray(&mut self) {
        self.tray_ok = unsafe { Shell_NotifyIconW(NIM_ADD, &self.tray_data()) } != 0;
    }
    fn balloon(&self, title: &str, text: &str) {
        if !self.tray_ok {
            return;
        }
        let mut data = self.tray_data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, text);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    fn tray_menu(&mut self, from_tray: bool) {
        self.menu_open = true;
        unsafe {
            let menu = CreatePopupMenu();
            for (id, label) in [
                (OPEN, "Open drives"),
                (SETTINGS, "Settings"),
                (REFRESH, "Refresh drives"),
            ] {
                AppendMenuW(menu, MF_STRING, id as usize, wide(label).as_ptr());
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            AppendMenuW(
                menu,
                MF_STRING,
                QUIT as usize,
                wide("Quit UsbEject\tCtrl+Q").as_ptr(),
            );
            let mut point = POINT::default();
            if from_tray {
                GetCursorPos(&mut point);
            } else {
                let mut r = RECT::default();
                GetWindowRect(self.control(MORE), &mut r);
                point = POINT {
                    x: r.right,
                    y: r.bottom,
                };
            }
            SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_RIGHTALIGN,
                point.x,
                point.y,
                0,
                self.hwnd,
                null(),
            );
            DestroyMenu(menu);
            if command != 0 {
                PostMessageW(self.hwnd, WM_COMMAND, command as usize, 0);
            }
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
        }
        self.menu_open = false;
    }
    fn panel_rows(&self) -> i32 {
        self.devices.len().min(3) as i32
    }
    fn panel_content_height(&self) -> i32 {
        if self.devices.is_empty() {
            154
        } else {
            self.panel_rows() * 82
        }
    }
    fn panel_client_height(&self) -> i32 {
        if self.devices.is_empty() {
            344 + if self.input(202).is_empty() { 0 } else { 64 }
        } else {
            112 + self.panel_content_height() + 200
        }
    }
    fn resize_panel(&self) {
        let dpi = self.window_dpi(self.hwnd);
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: 440 * dpi as i32 / 96,
            bottom: self.panel_client_height() * dpi as i32 / 96,
        };
        unsafe {
            AdjustWindowRectExForDpi(
                &mut bounds,
                GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32,
                0,
                WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
                dpi,
            );
            SetWindowPos(
                self.hwnd,
                null_mut(),
                0,
                0,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    fn position_panel(&self) {
        unsafe {
            let id = NOTIFYICONIDENTIFIER {
                cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..zeroed()
            };
            let mut anchor = RECT::default();
            let found = Shell_NotifyIconGetRect(&id, &mut anchor) >= 0;
            let monitor = if found {
                MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..zeroed()
            };
            GetMonitorInfoW(monitor, &mut info);
            let mut window = RECT::default();
            GetWindowRect(self.hwnd, &mut window);
            let width = window.right - window.left;
            let height = window.bottom - window.top;
            let gap = 12 * self.window_dpi(self.hwnd) as i32 / 96;
            let x = (if found {
                anchor.right - width
            } else {
                info.rcWork.right - width - gap
            })
            .clamp(
                info.rcWork.left + gap,
                (info.rcWork.right - width - gap).max(info.rcWork.left + gap),
            );
            let y = (if found {
                anchor.top - height - gap
            } else {
                info.rcWork.bottom - height - gap
            })
            .clamp(
                info.rcWork.top + gap,
                (info.rcWork.bottom - height - gap).max(info.rcWork.top + gap),
            );
            SetWindowPos(
                self.hwnd,
                null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    fn layout(&mut self, hwnd: HWND) {
        let index = self.theme_index(hwnd);
        if unsafe { IsWindowVisible(hwnd) } == 0
            && self.diagnostic.as_ref().is_none_or(|d| d.measure)
        {
            self.dirty[index] = true;
            return;
        }
        self.metrics.layouts[index] += 1;
        self.dirty[index] = false;
        let dpi = self.window_dpi(hwnd);
        if self.themes[index].dpi != dpi {
            self.themes[index] = Theme::new(dpi);
        }
        let theme = &self.themes[index];
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut rect);
        }
        let width = rect.right * 96 / dpi as i32;
        let height = rect.bottom * 96 / dpi as i32;
        let positions: Vec<(u16, i32, i32, i32, i32)> = if index == 0 {
            let bottom = 112 + self.panel_content_height();
            let status_y = bottom + if self.devices.is_empty() { 14 } else { 62 };
            vec![
                (200, 24, 18, width - 190, 34),
                (216, 24, 55, width - 48, 22),
                (REFRESH, width - 152, 20, 84, 34),
                (HIDE, width - 56, 20, 32, 34),
                (208, 24, 92, 200, 20),
                (203, width - 119, 92, 95, 20),
                (LIST, 20, 112, width - 40, self.panel_content_height()),
                (213, 24, 188, width - 48, 28),
                (214, 32, 222, width - 64, 38),
                (DETAILS, 24, bottom + 14, 112, 36),
                (CANCEL_MAIN, 24, bottom + 14, 112, 36),
                (EJECT, 146, bottom + 14, width - 170, 36),
                (
                    202,
                    24,
                    if self.devices.is_empty() {
                        274
                    } else {
                        status_y
                    },
                    width - 48,
                    56,
                ),
                (SETTINGS, 24, height - 56, 112, 34),
                (217, 147, height - 49, width - 245, 22),
                (MORE, width - 92, height - 56, 68, 34),
            ]
        } else if index == 1 {
            vec![
                (218, 28, 22, width - 56, 36),
                (207, 28, 63, width - 56, 24),
                (219, 28, 111, width - 56, 24),
                (STARTUP, 28, 148, width - 56, 28),
                (220, 51, 179, width - 79, 42),
                (CHECK, 28, 232, 180, 38),
                (INSTALL, 220, 232, width - 248, 38),
                (211, 28, 291, width - 56, 20),
                (204, 28, 317, width - 56, 88),
                (CANCEL, 28, height - 58, 110, 34),
                (CLOSE_SETTINGS, width - 128, height - 58, 100, 34),
            ]
        } else {
            vec![
                (221, 24, 18, width - 48, 34),
                (209, 24, 62, width - 48, 20),
                (201, 24, 88, width - 48, height - 385),
                (205, 24, height - 278, 120, 20),
                (ALIAS, 24, height - 252, width - 168, 32),
                (SAVE_ALIAS, width - 132, height - 252, 108, 32),
                (206, 24, height - 207, width - 48, 20),
                (FILE_PATH, 24, height - 182, width - 168, 32),
                (INSPECT, width - 132, height - 182, 108, 32),
                (210, 24, height - 139, width - 48, 20),
                (215, 24, height - 115, width - 48, 54),
                (CLOSE_DETAILS, width - 124, height - 48, 100, 32),
            ]
        };
        for (id, x, y, w, h) in positions {
            let control = self.control(id);
            unsafe {
                MoveWindow(control, theme.s(x), theme.s(y), theme.s(w), theme.s(h), 0);
            }
        }
        for (id, control) in &self.controls {
            if unsafe { GetParent(*control) } != hwnd {
                continue;
            }
            let font = match *id {
                200 | 218 | 221 => theme.title,
                213 => theme.heading,
                203 | 208 | 209 | 210 | 211 | 216 | 217 | 207 | 220 => theme.small,
                _ => theme.normal,
            };
            unsafe {
                SendMessageW(*control, WM_SETFONT, font as usize, 0);
            }
        }
        if index == 0 {
            let empty = self.devices.is_empty();
            for id in [LIST, DETAILS, EJECT] {
                unsafe {
                    ShowWindow(self.control(id), if empty { SW_HIDE } else { SW_SHOW });
                }
            }
            for id in [213, 214] {
                unsafe {
                    ShowWindow(self.control(id), if empty { SW_SHOW } else { SW_HIDE });
                }
            }
            for id in [208, 203] {
                unsafe {
                    ShowWindow(self.control(id), if empty { SW_HIDE } else { SW_SHOW });
                }
            }
            unsafe {
                ShowWindow(
                    self.control(202),
                    if !empty || !self.input(202).is_empty() {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
            unsafe {
                ShowWindow(
                    self.control(DETAILS),
                    if empty || self.ejecting {
                        SW_HIDE
                    } else {
                        SW_SHOW
                    },
                );
                ShowWindow(
                    self.control(CANCEL_MAIN),
                    if !empty && self.ejecting {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
            unsafe {
                SendMessageW(
                    self.control(LIST),
                    LB_SETITEMHEIGHT,
                    0,
                    theme.s(82) as isize,
                );
            }
        }
        unsafe {
            // Queue one coherent parent/child redraw after mutable UI state is
            // released. Native control callbacks must not paint with defaults.
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }
    fn paint(&mut self, hwnd: HWND, dc: HDC, printing: bool) {
        let index = self.theme_index(hwnd);
        if printing {
            self.metrics.prints[index] += 1;
        } else {
            self.metrics.paints[index] += 1;
        }
        if self.diagnostic.is_some() && self.paint_events.len() < 64 {
            let mut update = RECT::default();
            unsafe {
                GetClipBox(dc, &mut update);
            }
            self.paint_events.push(capture::PaintEvent {
                step: self.diagnostic_step,
                window: index,
                printing,
                rect: [update.left, update.top, update.right, update.bottom],
            });
        }
        let theme = &self.themes[index];
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut rect);
            FillRect(dc, &rect, theme.white);
        }
        let width = rect.right * 96 / theme.dpi as i32;
        if index == 0 {
            style::separator(dc, theme.s(84), rect.right);
            style::separator(dc, rect.bottom - theme.s(72), rect.right);
            if self.devices.is_empty() {
                let badge = theme.rect(width / 2 - 29, 112, 58, 58);
                style::rounded(dc, badge, theme.s(20), TINT, TINT);
                unsafe {
                    DrawIconEx(
                        dc,
                        theme.s(width / 2 - 16),
                        theme.s(125),
                        self.icon,
                        theme.s(32),
                        theme.s(32),
                        0,
                        null_mut(),
                        DI_NORMAL,
                    );
                }
            }
        } else if index == 1 {
            style::separator(dc, theme.s(95), rect.right);
        }
    }
    fn draw_item(&mut self, item: &DRAWITEMSTRUCT) {
        self.metrics.draws += 1;
        if self.diagnostic.is_some() && self.draw_events.len() < 64 {
            self.draw_events.push(capture::DrawEvent {
                step: self.diagnostic_step,
                control: item.CtlID,
                action: item.itemAction,
                state: item.itemState,
            });
        }
        let theme = &self.themes[self.theme_index(unsafe { GetParent(item.hwndItem) })];
        if item.CtlID != LIST as u32 {
            let id = item.CtlID as u16;
            let label = if id == HIDE {
                "×".to_owned()
            } else {
                self.input(id)
            };
            theme.button(item, id == EJECT || id == CHECK || id == INSTALL, &label);
            return;
        }
        if item.itemID == u32::MAX {
            return;
        }
        let Some(device) = self.devices.get(item.itemID as usize) else {
            return;
        };
        unsafe {
            FillRect(item.hDC, &item.rcItem, theme.white);
        }
        let mut card = item.rcItem;
        card.left += theme.s(4);
        card.right -= theme.s(4);
        card.top += theme.s(4);
        card.bottom -= theme.s(4);
        let selected = item.itemState & ODS_SELECTED != 0;
        style::rounded(
            item.hDC,
            card,
            theme.s(12),
            if selected { TINT } else { SURFACE },
            if selected { ACCENT } else { LINE },
        );
        unsafe {
            DrawIconEx(
                item.hDC,
                card.left + theme.s(12),
                card.top + theme.s(20),
                self.icon,
                theme.s(28),
                theme.s(28),
                0,
                null_mut(),
                DI_NORMAL,
            );
        }
        let textrect = RECT {
            left: card.left + theme.s(54),
            top: card.top + theme.s(12),
            right: card.right - theme.s(12),
            bottom: card.top + theme.s(36),
        };
        style::text_at(
            item.hDC,
            theme.heading,
            device.display_name(&self.settings.aliases),
            textrect,
            INK,
            DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        let mounts: Vec<_> = device
            .volumes
            .iter()
            .flat_map(|v| v.mounts.iter().map(|m| m.trim_end_matches('\\').to_owned()))
            .collect();
        let summary = format!(
            "{}  ·  {}{}",
            if mounts.is_empty() {
                device.bus.clone()
            } else {
                mounts.join("  ")
            },
            model::capacity_text(device.capacity),
            if device.blocked.is_some() {
                " · Protected"
            } else {
                ""
            }
        );
        let second = RECT {
            top: card.top + theme.s(40),
            bottom: card.bottom - theme.s(8),
            ..textrect
        };
        style::text_at(
            item.hDC,
            theme.small,
            &summary,
            second,
            MUTED,
            DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        if item.itemState & ODS_FOCUS != 0 {
            let mut r = card;
            r.left += 4;
            r.right -= 4;
            r.top += 4;
            r.bottom -= 4;
            unsafe {
                DrawFocusRect(item.hDC, &r);
            }
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            if self.tray_ok {
                Shell_NotifyIconW(NIM_DELETE, &self.tray_data());
            }
            if !self.notification.is_null() {
                UnregisterDeviceNotification(self.notification);
            }
            if !self.tray_icon.is_null() {
                DestroyIcon(self.tray_icon);
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}
fn copy_wide(target: &mut [u16], text: &str) {
    for (slot, code) in target.iter_mut().zip(text.encode_utf16()) {
        *slot = code;
    }
    target[target.len() - 1] = 0;
}

// Windows sends these messages synchronously from ShowWindow, EnableWindow,
// font changes and native backdrop painting. They must work while APP is
// already borrowed, and before the controller has finished initialization.
fn control_colors(dc: HDC, control: HWND) -> LRESULT {
    let id = unsafe { GetDlgCtrlID(control) };
    unsafe {
        SetTextColor(
            dc,
            if matches!(
                id,
                203 | 207 | 208 | 209 | 210 | 211 | 214 | 216 | 217 | 220
            ) {
                MUTED
            } else {
                INK
            },
        );
        SetBkColor(dc, WHITE);
        SetBkMode(dc, OPAQUE as i32);
        GetStockObject(WHITE_BRUSH) as LRESULT
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let role = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if matches!(
        message,
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN
    ) {
        return control_colors(wparam as HDC, lparam as HWND);
    }
    if message == WM_ERASEBKGND {
        let mut bounds = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut bounds);
            FillRect(wparam as HDC, &bounds, GetStockObject(WHITE_BRUSH));
        }
        return 1;
    }
    if message == REDRAW {
        unsafe {
            RedrawWindow(
                hwnd,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
        return 0;
    }
    if message == WM_PRINTCLIENT && role == 2 {
        // A themed native checkbox asks its parent to paint only the flat
        // backdrop beneath it. Service that local request directly, including
        // reentrant calls; it is not a reconstruction of the Settings scene.
        let mut clip = RECT::default();
        let mut bounds = RECT::default();
        let mut origin = POINT::default();
        unsafe {
            let checkbox = GetDlgItem(hwnd, STARTUP as i32);
            GetClipBox(wparam as HDC, &mut clip);
            GetClientRect(checkbox, &mut bounds);
            MapWindowPoints(checkbox, hwnd, &mut origin, 1);
        }
        if clip.right > clip.left
            && clip.bottom > clip.top
            && clip.left >= origin.x
            && clip.top >= origin.y
            && clip.right <= origin.x + bounds.right
            && clip.bottom <= origin.y + bounds.bottom
        {
            if unsafe { IsWindowVisible(hwnd) } != 0 {
                unsafe {
                    FillRect(wparam as HDC, &clip, GetStockObject(WHITE_BRUSH));
                }
                NATIVE_BACKDROPS.with(|count| count.set(count.get() + 1));
            }
            return 0;
        }
    }
    if message == WM_DESTROY {
        if role == 1 {
            unsafe {
                PostQuitMessage(0);
            }
        }
        return 0;
    }
    if message == WM_DEVICECHANGE
        && matches!(
            wparam as u32,
            DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | DBT_DEVNODES_CHANGED
        )
    {
        DEVICE_EPOCH.with(|e| {
            e.fetch_add(1, Ordering::AcqRel);
        });
        unsafe {
            SetTimer(hwnd, RESCAN_TIMER, 350, None);
        }
    }
    let handled = APP.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            if message == WM_DRAWITEM {
                unsafe {
                    PostMessageW(hwnd, REDRAW, 0, 0);
                }
                return Some(1);
            }
            return None;
        };
        let app = state.as_mut()?;
        if message == app.taskbar_created && app.diagnostic.is_none() {
            app.add_tray();
            return Some(0);
        }
        match message {
            WM_COMMAND => app.command((wparam & 0xffff) as u16, ((wparam >> 16) & 0xffff) as u16),
            WAKE => app.process_events(),
            RESTORE => app.restore(),
            WM_CLOSE => {
                if hwnd == app.hwnd {
                    app.hide_panel();
                } else {
                    unsafe {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                }
            }
            WM_ACTIVATE if role == 1 && wparam & 0xffff == WA_INACTIVE as usize => {
                let own_target =
                    lparam as HWND == app.settings_hwnd || lparam as HWND == app.details_hwnd;
                if app.tray_ok && !app.menu_open && !app.ejecting && !own_target {
                    app.hide_panel();
                }
            }
            WM_SIZE => app.layout(hwnd),
            WM_DPICHANGED => {
                let r = unsafe { &*(lparam as *const RECT) };
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                app.layout(hwnd);
            }
            WM_DEVICECHANGE if app.diagnostic.is_none() => {
                app.update_buttons();
            }
            WM_TIMER if wparam == RESCAN_TIMER => {
                unsafe {
                    KillTimer(hwnd, RESCAN_TIMER);
                }
                if app.diagnostic.is_none() {
                    app.scan();
                }
            }
            WM_TIMER if wparam == capture::TIMER => {
                app.diagnostic_tick();
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = unsafe { zeroed() };
                let dc = unsafe { BeginPaint(hwnd, &mut ps) };
                if app.can_paint(hwnd) {
                    app.paint(hwnd, dc, false);
                } else {
                    app.dirty[app.theme_index(hwnd)] = true;
                }
                unsafe {
                    EndPaint(hwnd, &ps);
                }
            }
            WM_PRINTCLIENT => {
                if app.can_paint(hwnd) {
                    app.paint(hwnd, wparam as HDC, true);
                }
            }
            WM_PRINT if !app.can_paint(hwnd) => return Some(0),
            WM_DRAWITEM => {
                if app.can_paint(hwnd) {
                    app.draw_item(unsafe { &*(lparam as *const DRAWITEMSTRUCT) });
                }
                return Some(1);
            }
            WM_MEASUREITEM => {
                let item = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
                if item.CtlID == LIST as u32 {
                    item.itemHeight = 82 * app.window_dpi(hwnd) / 96;
                    return Some(1);
                }
                return None;
            }
            TRAY => match lparam as u32 {
                WM_LBUTTONUP => app.restore(),
                WM_RBUTTONUP | WM_CONTEXTMENU => app.tray_menu(true),
                _ => {}
            },
            _ => return None,
        }
        Some(0)
    });
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}

// MAKEINTRESOURCEW(1) is a resource ordinal, deliberately not a memory pointer.
#[allow(clippy::manual_dangling_ptr)]
pub(crate) fn create_icon(instance: HINSTANCE, size: i32) -> HICON {
    let embedded = unsafe { LoadImageW(instance, 1usize as *const u16, IMAGE_ICON, size, size, 0) };
    if !embedded.is_null() {
        return embedded;
    }
    let mut pixels = crate::icon::pixels(size as usize);
    // CreateIcon's XOR bitmap is bottom-up, unlike the shared rasterizer.
    let stride = size as usize * 4;
    for y in 0..size as usize / 2 {
        let (top, bottom) = pixels.split_at_mut((size as usize - 1 - y) * stride);
        top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
    let mask = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    unsafe { CreateIcon(instance, size, size, 1, 32, mask.as_ptr(), pixels.as_ptr()) }
}

// Use the notification area's small-icon metric, independently of UI illustrations.
#[allow(clippy::manual_dangling_ptr)]
pub(crate) fn create_tray_icon(instance: HINSTANCE) -> HICON {
    let mut icon = null_mut();
    if unsafe { LoadIconMetric(instance, 1usize as *const u16, LIM_SMALL, &mut icon) } >= 0
        && !icon.is_null()
    {
        icon
    } else {
        create_icon(instance, unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16))
    }
}

fn child(
    parent: HWND,
    controls: &mut Vec<(u16, HWND)>,
    id: u16,
    class: &str,
    text: &str,
    flags: u32,
) -> HWND {
    let control = unsafe {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | flags,
            0,
            0,
            1,
            1,
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        )
    };
    controls.push((id, control));
    control
}
fn label(parent: HWND, controls: &mut Vec<(u16, HWND)>, id: u16, text: &str) {
    child(parent, controls, id, "STATIC", text, SS_LEFT | WS_GROUP);
}
fn button(parent: HWND, controls: &mut Vec<(u16, HWND)>, id: u16, text: &str) {
    child(
        parent,
        controls,
        id,
        "BUTTON",
        text,
        WS_TABSTOP | BS_OWNERDRAW as u32,
    );
}
fn edit(parent: HWND, controls: &mut Vec<(u16, HWND)>, id: u16, text: &str, readonly: bool) {
    child(
        parent,
        controls,
        id,
        "EDIT",
        text,
        WS_TABSTOP
            | if readonly {
                WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32
            } else {
                WS_BORDER | ES_AUTOHSCROLL as u32
            },
    );
}

pub fn run() {
    if installer::maybe_apply() {
        return;
    }
    let diagnostic = match capture::Options::parse() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = if diagnostic.is_none() {
            CreateMutexW(null(), 0, wide("Local\\UsbEject.Instance.v1").as_ptr())
        } else {
            null_mut()
        };
        if diagnostic.is_none() {
            if mutex.is_null() {
                return;
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let hwnd = FindWindowW(wide(CLASS).as_ptr(), null());
                if !hwnd.is_null() {
                    let mut pid = 0;
                    GetWindowThreadProcessId(hwnd, &mut pid);
                    AllowSetForegroundWindow(pid);
                    PostMessageW(hwnd, RESTORE, 0, 0);
                }
                CloseHandle(mutex);
                return;
            }
        }
        let instance = GetModuleHandleW(null());
        if let Some(options) = diagnostic.as_ref().filter(|d| d.screen == "icons") {
            let result = capture::save_icons(instance, &options.output);
            if let Err(error) = &result {
                eprintln!("{error}");
            }
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
        let icon = create_icon(instance, 32);
        let name = wide(CLASS);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(WHITE_BRUSH),
            lpszClassName: name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&class) == 0 {
            DestroyIcon(icon);
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            return;
        }
        let make_window = |title: &str, role: usize, owner: HWND, w: i32, h: i32, tool: bool| {
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT | if tool { WS_EX_TOOLWINDOW } else { 0 },
                name.as_ptr(),
                wide(title).as_ptr(),
                if tool {
                    WS_POPUP | WS_BORDER | WS_SYSMENU | WS_CLIPCHILDREN
                } else {
                    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
                },
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                owner,
                null_mut(),
                instance,
                role as *const _,
            );
            Theme::caption(hwnd);
            hwnd
        };
        let hwnd = make_window("UsbEject", 1, null_mut(), 440, 440, true);
        let settings_hwnd = make_window("UsbEject · Settings", 2, hwnd, 540, 520, false);
        let details_hwnd = make_window("UsbEject · Drive details", 3, hwnd, 620, 680, false);
        if hwnd.is_null() || settings_hwnd.is_null() || details_hwnd.is_null() {
            if !hwnd.is_null() {
                DestroyWindow(hwnd);
            }
            DestroyIcon(icon);
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            return;
        }
        let mut controls = Vec::new();
        label(hwnd, &mut controls, 200, "UsbEject");
        label(hwnd, &mut controls, 216, "External drives, safely removed.");
        button(hwnd, &mut controls, REFRESH, "Refresh");
        button(hwnd, &mut controls, HIDE, "Hide to tray");
        label(hwnd, &mut controls, 208, "EXTERNAL DRIVES");
        label(hwnd, &mut controls, 203, "Scanning…");
        child(
            hwnd,
            &mut controls,
            LIST,
            "LISTBOX",
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | LBS_NOTIFY as u32
                | LBS_NOINTEGRALHEIGHT as u32
                | LBS_OWNERDRAWFIXED as u32
                | LBS_HASSTRINGS as u32,
        );
        child(
            hwnd,
            &mut controls,
            213,
            "STATIC",
            "No external drives",
            windows_sys::Win32::System::SystemServices::SS_CENTER | WS_GROUP,
        );
        child(
            hwnd,
            &mut controls,
            214,
            "STATIC",
            "Connect a USB drive to see it here.",
            windows_sys::Win32::System::SystemServices::SS_CENTER,
        );
        button(hwnd, &mut controls, DETAILS, "Details");
        button(hwnd, &mut controls, CANCEL_MAIN, "Cancel");
        button(hwnd, &mut controls, EJECT, "Safely eject");
        label(hwnd, &mut controls, 202, "");
        button(hwnd, &mut controls, SETTINGS, "Settings");
        label(hwnd, &mut controls, 217, "Runs in your tray");
        button(hwnd, &mut controls, MORE, "More");

        label(settings_hwnd, &mut controls, 218, "Settings");
        label(
            settings_hwnd,
            &mut controls,
            207,
            &format!("UsbEject  ·  Version {}  ·  Windows x64", update::VERSION),
        );
        label(settings_hwnd, &mut controls, 219, "Updates");
        child(
            settings_hwnd,
            &mut controls,
            STARTUP,
            "BUTTON",
            "Check for updates on startup",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        );
        label(
            settings_hwnd,
            &mut controls,
            220,
            "Checks quietly in the background. Downloads and installation always need your action.",
        );
        button(settings_hwnd, &mut controls, CHECK, "Check for updates");
        button(
            settings_hwnd,
            &mut controls,
            INSTALL,
            "Download and install",
        );
        label(settings_hwnd, &mut controls, 211, "UPDATE STATUS");
        edit(
            settings_hwnd,
            &mut controls,
            204,
            "Loading update preferences…",
            true,
        );
        button(settings_hwnd, &mut controls, CANCEL, "Cancel");
        button(settings_hwnd, &mut controls, CLOSE_SETTINGS, "Close");

        label(details_hwnd, &mut controls, 221, "Drive details");
        label(details_hwnd, &mut controls, 209, "VOLUMES AND DEVICE");
        edit(
            details_hwnd,
            &mut controls,
            201,
            "Select a drive from the tray panel.",
            true,
        );
        label(details_hwnd, &mut controls, 205, "Display &label");
        edit(details_hwnd, &mut controls, ALIAS, "", false);
        button(details_hwnd, &mut controls, SAVE_ALIAS, "Save label");
        label(
            details_hwnd,
            &mut controls,
            206,
            "Inspect a busy file (full &path)",
        );
        edit(details_hwnd, &mut controls, FILE_PATH, "", false);
        button(details_hwnd, &mut controls, INSPECT, "Inspect file");
        label(details_hwnd, &mut controls, 210, "OPERATION STATUS");
        edit(
            details_hwnd,
            &mut controls,
            215,
            "Save your files before ejecting a drive.",
            true,
        );
        button(details_hwnd, &mut controls, CLOSE_DETAILS, "Close");
        let generation = DEVICE_EPOCH.with(Arc::clone);
        let disk_cancel = Arc::new(AtomicBool::new(false));
        let net_cancel = Arc::new(AtomicBool::new(false));
        let (disk, net, events) = workers(
            hwnd,
            generation.clone(),
            disk_cancel.clone(),
            net_cancel.clone(),
        );
        let notification = if diagnostic.is_none() {
            let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
                dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
                dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
                dbcc_classguid: windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK,
                ..zeroed()
            };
            RegisterDeviceNotificationW(
                hwnd,
                (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(),
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        } else {
            null_mut()
        };
        let dpi = diagnostic
            .as_ref()
            .map(|d| d.scale * 96 / 100)
            .unwrap_or(GetDpiForWindow(hwnd).max(96));
        for secondary in [settings_hwnd, details_hwnd] {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: if secondary == settings_hwnd {
                    540 * dpi as i32 / 96
                } else {
                    620 * dpi as i32 / 96
                },
                bottom: if secondary == settings_hwnd {
                    480 * dpi as i32 / 96
                } else {
                    640 * dpi as i32 / 96
                },
            };
            AdjustWindowRectExForDpi(
                &mut r,
                GetWindowLongPtrW(secondary, GWL_STYLE) as u32,
                0,
                WS_EX_CONTROLPARENT,
                dpi,
            );
            SetWindowPos(
                secondary,
                null_mut(),
                0,
                0,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let mut app = App {
            hwnd,
            settings_hwnd,
            details_hwnd,
            controls,
            themes: [Theme::new(dpi), Theme::new(dpi), Theme::new(dpi)],
            icon,
            tray_icon: create_tray_icon(instance),
            tray_ok: false,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            notification,
            disk,
            net,
            events,
            generation,
            disk_cancel,
            net_cancel,
            devices: vec![],
            snapshot_generation: 0,
            settings: Settings::default(),
            settings_ready: false,
            saving: false,
            disk_busy: false,
            ejecting: false,
            refresh_pending: false,
            net_busy: false,
            installing: false,
            handoff: false,
            available: None,
            menu_open: false,
            diagnostic,
            metrics: capture::Metrics::default(),
            dirty: [true; 3],
            diagnostic_step: 0,
            diagnostic_samples: vec![],
            diagnostic_failed: false,
            paint_events: vec![],
            draw_events: vec![],
        };
        if app.diagnostic.is_some() {
            app.prepare_diagnostic();
        } else {
            app.add_tray();
            if !app.tray_ok {
                app.status("The tray icon is unavailable. Closing this panel quits UsbEject.");
            }
            app.disk.try_send(DiskCommand::Initialize).ok();
        }
        app.resize_panel();
        let diagnostic = app.diagnostic.clone();
        APP.with(|state| *state.borrow_mut() = Some(app));
        if let Some(options) = diagnostic {
            if options.measure {
                let target = if options.screen == "settings" {
                    settings_hwnd
                } else {
                    hwnd
                };
                capture::position_measurement(target);
                ShowWindow(target, SW_SHOWNOACTIVATE);
                APP.with(|s| s.borrow_mut().as_mut().unwrap().layout(target));
                SetTimer(hwnd, capture::TIMER, 1000, None);
            } else {
                let target = if options.screen == "settings" {
                    settings_hwnd
                } else if options.screen == "details" {
                    details_hwnd
                } else {
                    hwnd
                };
                SetWindowPos(
                    target,
                    null_mut(),
                    -20000,
                    -20000,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                ShowWindow(target, SW_SHOWNOACTIVATE);
                APP.with(|s| s.borrow_mut().as_mut().unwrap().layout(target));
                let under_borrow = APP.with(|state| {
                    let state = state.borrow();
                    capture::palette_probe(target, &state.as_ref().unwrap().controls)
                });
                let controls = APP.with(|state| state.borrow().as_ref().unwrap().controls.clone());
                let outside_borrow = capture::palette_probe(target, &controls);
                let result = capture::save_client(
                    target,
                    &options.output,
                    serde_json::json!({"under_state_borrow":under_borrow,"outside_state_borrow":outside_borrow}),
                );
                DestroyWindow(hwnd);
                APP.with(|s| {
                    s.borrow_mut().take();
                });
                if let Err(error) = result {
                    eprintln!("{error}");
                    std::process::exit(3);
                }
                return;
            }
        } else if !std::env::args().any(|a| a == "--background")
            || APP.with(|s| s.borrow().as_ref().is_some_and(|app| !app.tray_ok))
        {
            APP.with(|s| s.borrow_mut().as_mut().unwrap().restore());
        }
        let mut message = MSG::default();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.message == WM_KEYDOWN
                && message.wParam == b'Q' as usize
                && GetKeyState(0x11) < 0
            {
                APP.with(|s| s.borrow_mut().as_mut().unwrap().quit());
                continue;
            }
            if message.message == WM_KEYDOWN && message.wParam == 0x1b {
                let root = GetAncestor(message.hwnd, GA_ROOT);
                APP.with(|state| {
                    let mut state = state.borrow_mut();
                    if let Some(app) = state.as_mut() {
                        if root == app.hwnd {
                            app.hide_panel();
                        } else {
                            ShowWindow(root, SW_HIDE);
                        }
                    }
                });
                continue;
            }
            let root = GetAncestor(message.hwnd, GA_ROOT);
            if IsDialogMessageW(root, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let failed = APP.with(|s| {
            s.borrow_mut()
                .take()
                .is_some_and(|app| app.diagnostic_failed)
        });
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
        if failed {
            std::process::exit(3);
        }
    }
}
