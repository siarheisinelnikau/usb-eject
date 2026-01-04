//! Original eject mark, rasterized independently at each native icon size.

pub const SIZES: [usize; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Top-down, premultiplied BGRA suitable for a 32-bit Windows icon.
pub fn pixels(size: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut color = [0u32; 4];
            // Small icons need clean edges without shrinking their visible footprint.
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f32 + (sx as f32 + 0.5) / 4.0) * 16.0 / size as f32;
                    let py = (y as f32 + (sy as f32 + 0.5) / 4.0) * 16.0 / size as f32;
                    let dx = (px - 8.0).abs() - 4.0;
                    let dy = (py - 8.0).abs() - 4.0;
                    let inside = dx.max(0.0).powi(2) + dy.max(0.0).powi(2) <= 9.0
                        && (1.0..15.0).contains(&px)
                        && (1.0..15.0).contains(&py);
                    if !inside {
                        continue;
                    }
                    let triangle =
                        (3.5..9.5).contains(&py) && (px - 8.0).abs() <= (py - 3.5) * 0.75;
                    let bar = (4.0..12.0).contains(&px) && (11.0..12.5).contains(&py);
                    let sample = if triangle || bar {
                        [255, 255, 255, 255]
                    } else {
                        [106, 119, 17, 255]
                    };
                    for (channel, value) in color.iter_mut().zip(sample) {
                        *channel += value;
                    }
                }
            }
            let offset = (y * size + x) * 4;
            for (channel, sum) in pixels[offset..offset + 4].iter_mut().zip(color) {
                *channel = ((sum + 8) / 16) as u8;
            }
        }
    }
    pixels
}
