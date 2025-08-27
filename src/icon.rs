/// Original disk-and-eject glyph in BGRA order, shared by build and runtime.
pub fn pixels() -> [u8; 32 * 32 * 4] {
    let mut pixels = [0u8; 32 * 32 * 4];
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
    pixels
}
