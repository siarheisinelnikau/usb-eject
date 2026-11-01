//! Opt-in, isolated developer diagnostics. No device/network/settings operations.
use serde::Serialize;
use std::{
    fs,
    io::BufWriter,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::null_mut,
};
use usb_eject::model::{Device, Identity, Volume};
use usb_eject::{i18n::Language, settings::ColorTheme};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::Threading::*, UI::WindowsAndMessaging::*,
};
pub const TIMER: usize = 912;

#[derive(Clone)]
pub struct Options {
    pub output: PathBuf,
    pub screen: String,
    pub scale: u32,
    pub measure: bool,
    pub language: Language,
    pub theme: ColorTheme,
}
impl Options {
    pub fn parse() -> Result<Option<Self>, String> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let Some(first) = args.first() else {
            return Ok(None);
        };
        if first == "--background" {
            return Ok(None);
        }
        if first != "--capture" && first != "--measure" {
            return Err("Use --capture OUTPUT.png [--screen empty|sample|error|settings|details|icons] [--scale 100|150|200] [--language en|ru|fr|de|es|it] [--theme light|dark], --measure OUTPUT.json, or --background".into());
        }
        let output = PathBuf::from(args.get(1).ok_or("Output path is required")?);
        if !output.is_absolute() {
            return Err("Diagnostic output must use an absolute path".into());
        }
        let mut result = Self {
            output,
            screen: "empty".into(),
            scale: 100,
            measure: first == "--measure",
            language: Language::English,
            theme: ColorTheme::Light,
        };
        let mut index = 2;
        while index < args.len() {
            let value = args
                .get(index + 1)
                .ok_or("Missing diagnostic option value")?
                .to_string_lossy();
            match args[index].to_string_lossy().as_ref() {
                "--screen" => result.screen = value.into_owned(),
                "--scale" => result.scale = value.parse().map_err(|_| "Invalid scale")?,
                "--language" => {
                    result.language =
                        Language::from_code(&value).ok_or("Unsupported diagnostic language")?
                }
                "--theme" => {
                    result.theme = match value.as_ref() {
                        "light" => ColorTheme::Light,
                        "dark" => ColorTheme::Dark,
                        _ => return Err("Unsupported diagnostic theme".into()),
                    }
                }
                _ => return Err("Unknown diagnostic option".into()),
            }
            index += 2;
        }
        if !matches!(
            result.screen.as_str(),
            "empty" | "sample" | "error" | "settings" | "details" | "icons"
        ) || !matches!(result.scale, 100 | 150 | 200)
            || (result.screen == "icons" && (result.measure || result.scale != 100))
        {
            return Err("Unsupported diagnostic screen or scale".into());
        }
        Ok(Some(result))
    }
}
#[derive(Clone, Copy, Default, Serialize)]
pub struct Metrics {
    pub paints: [u64; 3],
    pub layouts: [u64; 3],
    pub draws: u64,
    pub prints: [u64; 3],
    pub native_backdrops: u64,
}
#[derive(Serialize)]
pub struct PaintEvent {
    pub step: u32,
    pub window: usize,
    pub printing: bool,
    pub rect: [i32; 4],
}
#[derive(Serialize)]
pub struct DrawEvent {
    pub step: u32,
    pub control: u32,
    pub action: u32,
    pub state: u32,
}
#[derive(Serialize)]
pub struct Sample {
    step: u32,
    cpu_100ns: u64,
    wall_ms: u128,
    metrics: Metrics,
}
impl Sample {
    pub fn new(step: u32, metrics: Metrics) -> Self {
        let (mut created, mut exit, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = unsafe { zeroed() };
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exit,
                &mut kernel,
                &mut user,
            );
        }
        let value = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        Self {
            step,
            cpu_100ns: value(kernel) + value(user),
            wall_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            metrics,
        }
    }
}
pub fn write_measurement(
    path: &Path,
    samples: &[Sample],
    events: &[PaintEvent],
    draws: &[DrawEvent],
) -> Result<(), String> {
    let phases:Vec<_>=samples.chunks_exact(2).enumerate().map(|(index,pair)|{
        let a=&pair[0];let b=&pair[1];let seconds=(b.wall_ms-a.wall_ms)as f64/1000.;
        let cpu=(b.cpu_100ns-a.cpu_100ns)as f64/10_000_000.;
        let paint:u64=b.metrics.paints.iter().zip(a.metrics.paints).map(|(b,a)|b-a).sum();
        let layout:u64=b.metrics.layouts.iter().zip(a.metrics.layouts).map(|(b,a)|b-a).sum();
        let prints:u64=b.metrics.prints.iter().zip(a.metrics.prints).map(|(b,a)|b-a).sum();
        let logical=std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1);
        serde_json::json!({"state":(["visible_idle","hidden_with_data_update","restored_idle"][index]),"seconds":seconds,"cpu_seconds":cpu,"cpu_percent_one_core":cpu/seconds*100.,"cpu_percent_available_processors":cpu/seconds*100./logical as f64,"logical_processors_available":logical,"paints":paint,"layouts":layout,"owner_draws":b.metrics.draws-a.metrics.draws,"print_client_requests":prints,"native_checkbox_backdrops":b.metrics.native_backdrops-a.metrics.native_backdrops})
    }).collect();
    let report = serde_json::json!({"executable":std::env::current_exe().map_err(|e|e.to_string())?,"pid":std::process::id(),"backend":"Win32 native controls + CPU GDI; OS desktop composition remains enabled","fixture":"Two synthetic external drives; no device discovery/ejection, settings I/O or network","gpu_telemetry":"Not collected","samples":samples,"phases":phases,"paint_events":events,"draw_events":draws});
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())
}
pub fn position_measurement(hwnd: HWND) {
    // A newly shown native button beneath a stationary pointer can enter a
    // hover state. Keep this isolated idle sample away from the current pointer.
    unsafe {
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..zeroed()
        };
        GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds);
        let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        let points = [
            (info.rcWork.left + 24, info.rcWork.top + 24),
            (info.rcWork.right - w - 24, info.rcWork.bottom - h - 24),
            (info.rcWork.right - w - 24, info.rcWork.top + 24),
            (info.rcWork.left + 24, info.rcWork.bottom - h - 24),
        ];
        if let Some((x, y)) = points
            .into_iter()
            .find(|(x, y)| cursor.x < *x || cursor.x >= x + w || cursor.y < *y || cursor.y >= y + h)
        {
            SetWindowPos(
                hwnd,
                null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
pub fn palette_probe(parent: HWND, controls: &[(u16, HWND)]) -> serde_json::Value {
    let mut results = Vec::new();
    for (id, control) in controls {
        if unsafe { GetParent(*control) } != parent {
            continue;
        }
        let mut class = [0u16; 32];
        unsafe {
            GetClassNameW(*control, class.as_mut_ptr(), class.len() as i32);
        }
        let class = usb_eject::windows::from_wide(&class).to_ascii_uppercase();
        let message = match class.as_str() {
            "STATIC" => WM_CTLCOLORSTATIC,
            "EDIT" => WM_CTLCOLOREDIT,
            "BUTTON" => WM_CTLCOLORBTN,
            "COMBOBOX" => WM_CTLCOLORSTATIC,
            "LISTBOX" => WM_CTLCOLORLISTBOX,
            _ => continue,
        };
        let dc = unsafe { GetDC(*control) };
        unsafe {
            SetBkColor(dc, 0x00ff00ff);
        }
        let brush = unsafe { SendMessageW(parent, message, dc as usize, *control as isize) };
        let background = unsafe { GetBkColor(dc) };
        unsafe {
            ReleaseDC(*control, dc);
        }
        results.push(
            serde_json::json!({"control":id,"background":background,"brush_available":brush!=0}),
        );
    }
    serde_json::json!(results)
}
pub fn save_client(
    hwnd: HWND,
    path: &Path,
    palette: serde_json::Value,
    menu_probe: Result<serde_json::Value, String>,
) -> Result<(), String> {
    let menu_probe = menu_probe?;
    unsafe {
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r);
        let (w, h) = (r.right, r.bottom);
        if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            return Err("Invalid capture dimensions".into());
        }
        let dc = GetDC(hwnd);
        let memory = CreateCompatibleDC(dc);
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if bitmap.is_null() || memory.is_null() || pixels.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !memory.is_null() {
                DeleteDC(memory);
            }
            ReleaseDC(hwnd, dc);
            return Err("Cannot allocate capture bitmap".into());
        }
        let old = SelectObject(memory, bitmap);
        let mut fonts = Vec::new();
        let mut control = GetWindow(hwnd, GW_CHILD);
        while !control.is_null() {
            let font = SendMessageW(control, WM_GETFONT, 0, 0) as HFONT;
            if !font.is_null() {
                let previous = SelectObject(memory, font);
                let mut face = [0u16; 128];
                GetTextFaceW(memory, face.len() as i32, face.as_mut_ptr());
                fonts.push(serde_json::json!({"control":GetDlgCtrlID(control),"actual_face":usb_eject::windows::from_wide(&face)}));
                SelectObject(memory, previous);
            }
            control = GetWindow(control, GW_HWNDNEXT);
        }
        let printed = windows_sys::Win32::Storage::Xps::PrintWindow(
            hwnd,
            memory,
            windows_sys::Win32::Storage::Xps::PW_CLIENTONLY,
        );
        GdiFlush();
        let raw = std::slice::from_raw_parts(pixels.cast::<u8>(), (w * h * 4) as usize);
        let mut rgba = Vec::with_capacity(raw.len());
        for pixel in raw.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
        SelectObject(memory, old);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(hwnd, dc);
        if printed == 0 {
            return Err("Native client capture failed".into());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temp = path.with_extension("png.tmp");
        let file = fs::File::create(&temp).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        usb_eject::settings::replace(&temp, path).map_err(|e| e.to_string())?;
        fs::write(path.with_extension("capture.json"), serde_json::to_vec_pretty(&serde_json::json!({"client_width":w,"client_height":h,"native_caption":GetWindowLongPtrW(hwnd,GWL_STYLE)as u32 & WS_CAPTION == WS_CAPTION,"backend":"Native GDI PrintWindow client capture","fonts":fonts,"reentrant_palette":palette,"menu_dispatch_probe":menu_probe})).unwrap()).map_err(|e|e.to_string())
    }
}

fn icon_footprint(icon: HICON) -> Result<serde_json::Value, String> {
    unsafe {
        let mut info: ICONINFO = zeroed();
        if icon.is_null() || GetIconInfo(icon, &mut info) == 0 {
            return Err("Cannot inspect native icon".into());
        }
        let result = (|| {
            let mut bitmap: BITMAP = zeroed();
            if GetObjectW(
                info.hbmColor,
                size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            ) == 0
            {
                return Err("Cannot inspect icon bitmap".to_owned());
            }
            let (w, h) = (bitmap.bmWidth, bitmap.bmHeight);
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let mut dib: BITMAPINFO = zeroed();
            dib.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            dib.bmiHeader.biWidth = w;
            dib.bmiHeader.biHeight = -h;
            dib.bmiHeader.biPlanes = 1;
            dib.bmiHeader.biBitCount = 32;
            let dc = CreateCompatibleDC(null_mut());
            let rows = GetDIBits(
                dc,
                info.hbmColor,
                0,
                h as u32,
                pixels.as_mut_ptr().cast(),
                &mut dib,
                DIB_RGB_COLORS,
            );
            DeleteDC(dc);
            if rows != h {
                return Err("Cannot read icon alpha".into());
            }
            let mut bounds = [w, h, 0, 0];
            for y in 0..h {
                for x in 0..w {
                    if pixels[((y * w + x) * 4 + 3) as usize] >= 128 {
                        bounds[0] = bounds[0].min(x);
                        bounds[1] = bounds[1].min(y);
                        bounds[2] = bounds[2].max(x + 1);
                        bounds[3] = bounds[3].max(y + 1);
                    }
                }
            }
            if bounds[2] - bounds[0] < w * 7 / 8 || bounds[3] - bounds[1] < h * 7 / 8 {
                return Err(format!("Icon artwork is undersized: {w}x{h}, {bounds:?}"));
            }
            Ok(serde_json::json!({"width":w,"height":h,"alpha_128_bounds":bounds}))
        })();
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
        result
    }
}

/// Native resource proof, not a screenshot of Explorer's notification area.
#[allow(clippy::manual_dangling_ptr)]
pub fn save_icons(instance: HINSTANCE, path: &Path) -> Result<(), String> {
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = 400;
        info.bmiHeader.biHeight = -176;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if dc.is_null() || bitmap.is_null() || pixels.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            return Err("Cannot allocate native icon capture".into());
        }
        let previous = SelectObject(dc, bitmap);
        let result = (|| {
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 400,
                bottom: 176,
            };
            FillRect(dc, &bounds, GetStockObject(WHITE_BRUSH));
            let dark = CreateSolidBrush(0x00202020);
            FillRect(
                dc,
                &RECT {
                    top: 48,
                    bottom: 112,
                    ..bounds
                },
                dark,
            );
            DeleteObject(dark);
            let theme = crate::ui_style::Theme::new(96);
            let mut resources = Vec::new();
            for (column, size) in crate::icon::SIZES.into_iter().enumerate() {
                let icon = LoadImageW(
                    instance,
                    1usize as *const u16,
                    IMAGE_ICON,
                    size as i32,
                    size as i32,
                    0,
                );
                let footprint = icon_footprint(icon);
                if column < 4 && !icon.is_null() {
                    let x = column as i32 * 100;
                    crate::ui_style::text_at(
                        dc,
                        theme.normal,
                        &format!("{size} px"),
                        RECT {
                            left: x,
                            top: 12,
                            right: x + 100,
                            bottom: 36,
                        },
                        crate::ui_style::INK,
                        DT_CENTER | DT_SINGLELINE,
                    );
                    for center_y in [80, 144] {
                        if DrawIconEx(
                            dc,
                            x + (100 - size as i32) / 2,
                            center_y - size as i32 / 2,
                            icon,
                            size as i32,
                            size as i32,
                            0,
                            null_mut(),
                            DI_NORMAL,
                        ) == 0
                        {
                            DestroyIcon(icon);
                            return Err("Native icon rendering failed".into());
                        }
                    }
                }
                if !icon.is_null() {
                    DestroyIcon(icon);
                }
                let footprint = footprint?;
                if footprint["width"] != size || footprint["height"] != size {
                    return Err(
                        "Embedded icon dimensions do not match the requested resource".into(),
                    );
                }
                resources.push(footprint);
            }
            let tray_icon = crate::ui::create_tray_icon(instance);
            let tray = icon_footprint(tray_icon);
            if !tray_icon.is_null() {
                DestroyIcon(tray_icon);
            }
            let tray = tray?;
            if tray["width"] != GetSystemMetrics(SM_CXSMICON)
                || tray["height"] != GetSystemMetrics(SM_CYSMICON)
            {
                return Err("Tray icon does not match the native small-icon metric".into());
            }
            GdiFlush();
            let mut rgba = Vec::with_capacity(400 * 176 * 4);
            for pixel in
                std::slice::from_raw_parts(pixels.cast::<u8>(), 400 * 176 * 4).chunks_exact(4)
            {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let file = fs::File::create(path).map_err(|e| e.to_string())?;
            let mut encoder = png::Encoder::new(BufWriter::new(file), 400, 176);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
            writer.finish().map_err(|e| e.to_string())?;
            fs::write(path.with_extension("capture.json"),serde_json::to_vec_pretty(&serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"backend":"Native embedded HICON / GDI DrawIconEx; no Explorer or desktop capture","sizes":resources,"tray_metric":tray})).unwrap()).map_err(|e|e.to_string())
        })();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        result
    }
}
pub fn sample_devices() -> Vec<Device> {
    [
        ("Travel SSD", "F:\\", "Projects", 512u64 << 30, 148u64 << 30),
        ("Photo backup", "G:\\", "Archive", 2u64 << 40, 893u64 << 30),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, mount, label, total, free))| {
        let id = format!("SAMPLE-ONLY-{i}");
        Device {
            identity: Identity {
                instance_id: id.clone(),
                parent_instance_id: format!("SAMPLE-PARENT-{i}"),
                interface_path: "SAMPLE-NOT-A-DEVICE".into(),
                disk_number: 100 + i as u32,
                serial: "CAPTURE-FIXTURE".into(),
                volumes: vec![id.clone()],
            },
            name: name.into(),
            bus: "USB".into(),
            capacity: Some(total),
            volumes: vec![Volume {
                id,
                mounts: vec![mount.into()],
                label: label.into(),
                capacity: Some(total),
                free: Some(free),
                disk_numbers: vec![100 + i as u32],
            }],
            blocked: None,
        }
    })
    .collect()
}
