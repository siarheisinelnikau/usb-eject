#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=resources/UsbEject.manifest");
    println!("cargo:rerun-if-changed=src/icon.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        Ok("x86_64"),
        "Only Windows x64 distribution is supported in v0.1"
    );
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("UsbEject.ico");
    let mut bytes = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
    bytes.extend_from_slice(&(40u32 + 4096 + 128).to_le_bytes());
    bytes.extend_from_slice(&22u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&32i32.to_le_bytes());
    bytes.extend_from_slice(&64i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 24]);
    let pixels = icon::pixels();
    for row in (0..32).rev() {
        bytes.extend_from_slice(&pixels[row * 128..(row + 1) * 128]);
    }
    bytes.extend_from_slice(&[0u8; 128]);
    std::fs::write(&output, bytes).expect("write generated application icon");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let manifest = std::fs::read_to_string("resources/UsbEject.manifest")
        .unwrap()
        .replace("@VERSION@", &version);
    // Embed the file verbatim. Inline RC string manifests can introduce leading
    // whitespace before the XML declaration, which breaks activation contexts.
    let manifest_path = output.with_extension("manifest");
    std::fs::write(&manifest_path, manifest).expect("write versioned manifest");
    let components: Vec<u64> = version.split('.').map(|n| n.parse().unwrap()).collect();
    let numeric = (components[0] << 48) | (components[1] << 32) | (components[2] << 16);
    let mut resource = winresource::WindowsResource::new();
    resource
        .set("ProductName", "UsbEject")
        .set("FileDescription", "UsbEject external-drive ejection")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set("OriginalFilename", &format!("UsbEject-v{version}.exe"))
        .set("LegalCopyright", "UsbEject contributors. MIT license.")
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric)
        .set_manifest_file(manifest_path.to_str().unwrap())
        .set_icon(output.to_str().unwrap());
    resource
        .compile_for("UsbEject")
        .expect("compile Windows resources using the installed Windows SDK");
}
