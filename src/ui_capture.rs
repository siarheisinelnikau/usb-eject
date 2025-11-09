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
            return Err("Use --capture OUTPUT.png [--screen empty|sample|error|settings|details] [--scale 100|150|200], --measure OUTPUT.json, or --background".into());
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
                _ => return Err("Unknown diagnostic option".into()),
            }
            index += 2;
        }
        if !matches!(
            result.screen.as_str(),
            "empty" | "sample" | "error" | "settings" | "details"
        ) || !matches!(result.scale, 100 | 150 | 200)
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
}
#[derive(Serialize)]
pub struct PaintEvent {
    pub step: u32,
    pub window: usize,
    pub printing: bool,
    pub rect: [i32; 4],
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
) -> Result<(), String> {
    let phases:Vec<_>=samples.chunks_exact(2).enumerate().map(|(index,pair)|{
        let a=&pair[0];let b=&pair[1];let seconds=(b.wall_ms-a.wall_ms)as f64/1000.;
        let cpu=(b.cpu_100ns-a.cpu_100ns)as f64/10_000_000.;
        let paint:u64=b.metrics.paints.iter().zip(a.metrics.paints).map(|(b,a)|b-a).sum();
        let layout:u64=b.metrics.layouts.iter().zip(a.metrics.layouts).map(|(b,a)|b-a).sum();
        let prints:u64=b.metrics.prints.iter().zip(a.metrics.prints).map(|(b,a)|b-a).sum();
        let logical=std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1);
        serde_json::json!({"state":(["visible_idle","hidden_with_data_update","restored_idle"][index]),"seconds":seconds,"cpu_seconds":cpu,"cpu_percent_one_core":cpu/seconds*100.,"cpu_percent_available_processors":cpu/seconds*100./logical as f64,"logical_processors_available":logical,"paints":paint,"layouts":layout,"owner_draws":b.metrics.draws-a.metrics.draws,"print_client_requests":prints})
    }).collect();
    let report = serde_json::json!({"executable":std::env::current_exe().map_err(|e|e.to_string())?,"pid":std::process::id(),"backend":"Win32 native controls + CPU GDI; OS desktop composition remains enabled","fixture":"Two synthetic external drives; no device discovery/ejection, settings I/O or network","gpu_telemetry":"Not collected","samples":samples,"phases":phases,"paint_events":events});
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())
}
pub fn save_client(hwnd: HWND, path: &Path) -> Result<(), String> {
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
        fs::write(path.with_extension("capture.json"), serde_json::to_vec_pretty(&serde_json::json!({"client_width":w,"client_height":h,"backend":"Native GDI PrintWindow client capture","fonts":fonts})).unwrap()).map_err(|e|e.to_string())
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
