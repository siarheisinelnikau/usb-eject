//! Event-driven Win32 UI. All storage, settings and network I/O runs on bounded workers.
use std::{
    cell::RefCell,
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
        Controls::*, HiDpi::*, Input::KeyboardAndMouse::EnableWindow, Shell::*,
        WindowsAndMessaging::*,
    },
};

const WAKE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const RESTORE: u32 = WM_APP + 3;
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
    controls: Vec<(u16, HWND)>,
    font: HFONT,
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
    armed: Option<(Identity, u64)>,
    settings: Settings,
    settings_ready: bool,
    saving: bool,
    disk_busy: bool,
    ejecting: bool,
    refresh_pending: bool,
    net_busy: bool,
    installing: bool,
    available: Option<Available>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DEVICE_EPOCH: Arc<AtomicU64> = Arc::new(AtomicU64::new(1));
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
    fn status(&self, text: &str) {
        self.text(202, text);
    }
    fn selected(&self) -> Option<&Device> {
        let index = unsafe { SendMessageW(self.control(LIST), LB_GETCURSEL, 0, 0) };
        if index < 0 {
            None
        } else {
            self.devices.get(index as usize)
        }
    }
    fn disarm(&mut self) {
        self.armed = None;
        self.text(EJECT, "Review ejection");
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
        self.enabled(CANCEL, self.disk_busy || self.installing);
        self.enabled(
            SAVE_ALIAS,
            self.settings_ready && !self.saving && !self.installing && self.selected().is_some(),
        );
        self.enabled(
            STARTUP,
            self.settings_ready && !self.saving && !self.installing,
        );
        self.enabled(CHECK, !self.net_busy);
        self.enabled(
            INSTALL,
            self.available.is_some() && !self.net_busy && !self.disk_busy && !self.saving,
        );
    }
    fn selected_changed(&mut self) {
        self.disarm();
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
        if let Some(index) = previous
            .and_then(|id| self.devices.iter().position(|d| d.identity == id))
            .or_else(|| (!self.devices.is_empty()).then_some(0))
        {
            unsafe {
                SendMessageW(self.control(LIST), LB_SETCURSEL, index, 0);
            }
        }
        self.selected_changed();
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
            self.disarm();
            self.text(203, "Discovering external storage…");
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
                if self.armed.as_ref() != Some(&(device.identity.clone(), epoch)) {
                    self.armed = Some((device.identity.clone(), epoch));
                    self.text(EJECT, "Confirm eject");
                    self.status(&format!("Ready to eject {} and ALL {} volume(s). Save files and close applications using this drive. Choose Confirm eject to ask Windows for safe removal.", device.display_name(&self.settings.aliases), device.volumes.len()));
                } else if self
                    .disk
                    .try_send(DiskCommand::Eject(device.identity, epoch))
                    .is_ok()
                {
                    self.disk_cancel.store(false, Ordering::Release);
                    self.disk_busy = true;
                    self.ejecting = true;
                    self.disarm();
                    self.status("Validating device identity and requesting safe removal… Cancel can stop preparation; an active Windows PnP request cannot be undone.");
                }
            }
            CANCEL => {
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
                            }
                        }
                        Err(error) => {
                            self.settings.check_updates_on_startup = false;
                            self.status(&format!("{error}. Settings editing and startup update checks are disabled for this session."));
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
                    if epoch == self.generation.load(Ordering::Acquire) {
                        self.snapshot_generation = epoch;
                        match result {
                            Ok(devices) => {
                                self.text(
                                    203,
                                    &format!("{} external physical device(s)", devices.len()),
                                );
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
                            self.settings = value;
                            self.populate(self.devices.clone());
                            self.status("Settings saved. Display labels only affect UsbEject; volume labels are unchanged.");
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
                Event::Installed(result) => {
                    self.net_busy = false;
                    self.installing = false;
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
    fn restore(&self) {
        unsafe {
            ShowWindow(
                self.hwnd,
                if IsIconic(self.hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(self.hwnd);
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
        copy_wide(&mut data.szTip, "UsbEject — external drive safety");
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
    fn tray_menu(&self) {
        unsafe {
            let menu = CreatePopupMenu();
            for (id, label) in [
                (OPEN, "Open UsbEject"),
                (REFRESH, "Refresh devices"),
                (QUIT, "Quit"),
            ] {
                AppendMenuW(menu, MF_STRING, id as usize, wide(label).as_ptr());
            }
            let mut point = POINT::default();
            GetCursorPos(&mut point);
            SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                self.hwnd,
                null(),
            );
            DestroyMenu(menu);
            // Queue commands instead of reentering App through a synchronous callback.
            if command != 0 {
                PostMessageW(self.hwnd, WM_COMMAND, command as usize, 0);
            }
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
        }
    }
    fn layout(&mut self) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        let scale = |v: i32| v * dpi as i32 / 96;
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(self.hwnd, &mut rect);
        }
        let width = rect.right * 96 / dpi as i32;
        let height = rect.bottom * 96 / dpi as i32;
        let left = 310;
        let right = width - left - 48;
        let body = (height - 410).max(190);
        let y = 60 + body;
        let positions = [
            (200, 18, 14, width - 36, 34),
            (LIST, 18, 60, left, body),
            (201, left + 30, 60, right, body),
            (REFRESH, 18, y + 10, 100, 30),
            (EJECT, 128, y + 10, 150, 30),
            (CANCEL, 288, y + 10, 90, 30),
            (203, 390, y + 12, width - 408, 26),
            (205, 18, y + 50, 135, 24),
            (ALIAS, 155, y + 46, width - 300, 28),
            (SAVE_ALIAS, width - 132, y + 46, 114, 29),
            (206, 18, y + 87, 135, 24),
            (FILE_PATH, 155, y + 83, width - 300, 28),
            (INSPECT, width - 132, y + 83, 114, 29),
            (202, 18, y + 122, width - 36, 88),
            (207, 18, y + 221, width - 36, 24),
            (STARTUP, 18, y + 249, 325, 26),
            (CHECK, width - 350, y + 246, 155, 30),
            (INSTALL, width - 185, y + 246, 167, 30),
            (204, 18, y + 282, width - 36, 47),
        ];
        for (id, x, y, w, h) in positions {
            unsafe {
                MoveWindow(self.control(id), scale(x), scale(y), scale(w), scale(h), 1);
            }
        }
        let font = unsafe {
            CreateFontW(
                -scale(15),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            )
        };
        if !font.is_null() {
            for (_, control) in &self.controls {
                unsafe {
                    SendMessageW(*control, WM_SETFONT, font as usize, 1);
                }
            }
            if !self.font.is_null() {
                unsafe {
                    DeleteObject(self.font);
                }
            }
            self.font = font;
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
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            if !self.tray_icon.is_null() {
                DestroyIcon(self.tray_icon);
            }
        }
    }
}
fn copy_wide(target: &mut [u16], text: &str) {
    for (slot, code) in target
        .iter_mut()
        .take(text.encode_utf16().count())
        .zip(text.encode_utf16())
    {
        *slot = code;
    }
    target[target.len() - 1] = 0;
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Native menus run a nested message loop. Invalidate identities even while
    // App is borrowed by that loop; never lose a hot-plug notification.
    if message == WM_DEVICECHANGE
        && matches!(
            wparam as u32,
            DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | DBT_DEVNODES_CHANGED
        )
    {
        DEVICE_EPOCH.with(|epoch| {
            epoch.fetch_add(1, Ordering::AcqRel);
        });
        unsafe {
            SetTimer(hwnd, RESCAN_TIMER, 350, None);
        }
    }
    if message == WM_DESTROY {
        unsafe {
            PostQuitMessage(0);
        }
        return 0;
    }
    if message == WM_GETMINMAXINFO {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
        let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
        info.ptMinTrackSize = POINT {
            x: 900 * dpi / 96,
            y: 810 * dpi / 96,
        };
        return 0;
    }
    // Native controls can synchronously notify their parent. RefCell avoids
    // aliased mutable UI state; irrelevant nested notifications use DefWindowProc.
    let handled = APP.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return false;
        };
        let Some(app) = state.as_mut() else {
            return false;
        };
        if message == app.taskbar_created {
            app.add_tray();
            return true;
        }
        match message {
            WM_COMMAND => app.command((wparam & 0xffff) as u16, ((wparam >> 16) & 0xffff) as u16),
            WAKE => app.process_events(),
            RESTORE => app.restore(),
            WM_CLOSE => {
                if app.tray_ok {
                    unsafe {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                } else {
                    app.quit();
                }
            }
            WM_SIZE => app.layout(),
            WM_DPICHANGED => {
                let rect = unsafe { &*(lparam as *const RECT) };
                unsafe {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                app.layout();
            }
            WM_DEVICECHANGE
                if matches!(
                    wparam as u32,
                    DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | DBT_DEVNODES_CHANGED
                ) =>
            {
                app.disarm();
                app.update_buttons();
            }
            WM_TIMER if wparam == RESCAN_TIMER => {
                unsafe {
                    KillTimer(hwnd, RESCAN_TIMER);
                }
                app.scan();
            }
            TRAY => match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => app.restore(),
                WM_RBUTTONUP | WM_CONTEXTMENU => app.tray_menu(),
                _ => {}
            },
            _ => return false,
        }
        true
    });
    if handled {
        0
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn create_icon(instance: HINSTANCE) -> HICON {
    // Original 32x32 disk-and-eject glyph, generated locally without external assets.
    let mut pixels = [0u8; 32 * 32 * 4];
    let mask = [0u8; 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let disk = (5..27).contains(&x) && (4..28).contains(&y);
            let arrow = (11..22).contains(&y) && (x as i32 - 16).abs() <= (y as i32 - 11) / 2;
            let bar = (12..=20).contains(&x) && (23..=25).contains(&y);
            let color = if arrow || bar {
                [220, 255, 230, 255]
            } else if disk {
                [112, 70, 28, 255]
            } else {
                [0, 0, 0, 0]
            };
            pixels[i..i + 4].copy_from_slice(&color);
        }
    }
    unsafe { CreateIcon(instance, 32, 32, 1, 32, mask.as_ptr(), pixels.as_ptr()) }
}

pub fn run() {
    if installer::maybe_apply() {
        return;
    }
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = CreateMutexW(null(), 0, wide("Local\\UsbEject.Instance.v1").as_ptr());
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
        let instance = GetModuleHandleW(null());
        let icon = create_icon(instance);
        let class_name = wide(CLASS);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&class) == 0 {
            DestroyIcon(icon);
            CloseHandle(mutex);
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_name.as_ptr(),
            wide("UsbEject — External storage").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            980,
            850,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            DestroyIcon(icon);
            CloseHandle(mutex);
            return;
        }
        let mut controls = Vec::new();
        let mut add = |id: u16, class: &str, text: &str, style: u32| {
            let control = CreateWindowExW(
                if class == "EDIT" || class == "LISTBOX" {
                    WS_EX_CLIENTEDGE
                } else {
                    0
                },
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                10,
                10,
                hwnd,
                id as usize as HMENU,
                instance,
                null(),
            );
            controls.push((id, control));
        };
        add(
            200,
            "STATIC",
            "UsbEject   /   Safely remove an external physical drive",
            SS_LEFT,
        );
        add(
            LIST,
            "LISTBOX",
            "",
            WS_TABSTOP | WS_VSCROLL | WS_HSCROLL | LBS_NOTIFY as u32 | LBS_NOINTEGRALHEIGHT as u32,
        );
        add(
            201,
            "EDIT",
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
        );
        for (id, text) in [
            (REFRESH, "Refresh"),
            (EJECT, "Review ejection"),
            (CANCEL, "Cancel"),
            (SAVE_ALIAS, "Save label"),
            (INSPECT, "Inspect file"),
            (CHECK, "Check for updates"),
            (INSTALL, "Download and install"),
        ] {
            add(id, "BUTTON", text, WS_TABSTOP | BS_PUSHBUTTON as u32);
        }
        add(ALIAS, "EDIT", "", WS_TABSTOP | ES_AUTOHSCROLL as u32);
        add(FILE_PATH, "EDIT", "", WS_TABSTOP | ES_AUTOHSCROLL as u32);
        add(
            202,
            "EDIT",
            "Save your work before ejection. Review the selected device, then confirm. Closing this window keeps UsbEject in the tray; choose Quit from its menu to exit.",
            WS_TABSTOP | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32,
        );
        add(203, "STATIC", "Loading…", SS_LEFT);
        add(
            204,
            "EDIT",
            "Startup checks are enabled by default. The planned repository may not be published yet.",
            WS_TABSTOP | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32,
        );
        add(205, "STATIC", "Display label:", SS_LEFT);
        add(206, "STATIC", "File in use (path):", SS_LEFT);
        add(
            207,
            "STATIC",
            &format!(
                "Settings / Updates    ·    Installed version {}    ·    Windows x64",
                update::VERSION
            ),
            SS_LEFT,
        );
        add(
            STARTUP,
            "BUTTON",
            "Check for updates on startup",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        );
        let generation = DEVICE_EPOCH.with(Arc::clone);
        let disk_cancel = Arc::new(AtomicBool::new(false));
        let net_cancel = Arc::new(AtomicBool::new(false));
        let (disk, net, events) = workers(
            hwnd,
            generation.clone(),
            disk_cancel.clone(),
            net_cancel.clone(),
        );
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
            dbcc_classguid: windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK,
            ..zeroed()
        };
        let notification = RegisterDeviceNotificationW(
            hwnd,
            (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let mut app = App {
            hwnd,
            controls,
            font: null_mut(),
            tray_icon: icon,
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
            armed: None,
            settings: Settings::default(),
            settings_ready: false,
            saving: false,
            disk_busy: false,
            ejecting: false,
            refresh_pending: false,
            net_busy: false,
            installing: false,
            available: None,
        };
        app.add_tray();
        app.layout();
        app.selected_changed();
        app.disk.try_send(DiskCommand::Initialize).ok();
        if !app.tray_ok {
            app.status("Tray icon could not be created. Closing the window will quit UsbEject.");
        }
        if app.notification.is_null() {
            app.status("Device notifications could not be registered. Use Refresh after connecting or removing storage.");
        }
        APP.with(|state| *state.borrow_mut() = Some(app));
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut message = MSG::default();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        APP.with(|state| {
            state.borrow_mut().take();
        });
        CloseHandle(mutex);
    }
}
