#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod ui;

fn main() {
    #[cfg(windows)]
    ui::run();
    #[cfg(not(windows))]
    eprintln!("UsbEject supports Windows 10/11 x64 only.");
}
