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
    let mut bytes = vec![0, 0, 1, 0, icon::SIZES.len() as u8, 0];
    let mut offset = 6 + icon::SIZES.len() * 16;
    let mut images = Vec::new();
    for size in icon::SIZES {
        let mask_stride = size.div_ceil(32) * 4;
        let image_size = 40 + size * size * 4 + mask_stride * size;
        // A zero dimension in an ICO directory represents 256 pixels.
        bytes.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&(image_size as u32).to_le_bytes());
        bytes.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image_size;
        images.extend_from_slice(&40u32.to_le_bytes());
        images.extend_from_slice(&(size as i32).to_le_bytes());
        images.extend_from_slice(&(size as i32 * 2).to_le_bytes());
        images.extend_from_slice(&1u16.to_le_bytes());
        images.extend_from_slice(&32u16.to_le_bytes());
        images.extend_from_slice(&[0u8; 24]);
        let pixels = icon::pixels(size);
        for row in (0..size).rev() {
            images.extend_from_slice(&pixels[row * size * 4..(row + 1) * size * 4]);
        }
        // Keep the legacy mask transparent wherever the alpha channel is zero.
        for row in (0..size).rev() {
            let mut mask = vec![0u8; mask_stride];
            for x in 0..size {
                if pixels[(row * size + x) * 4 + 3] == 0 {
                    mask[x / 8] |= 0x80 >> (x % 8);
                }
            }
            images.extend_from_slice(&mask);
        }
    }
    bytes.extend_from_slice(&images);
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
