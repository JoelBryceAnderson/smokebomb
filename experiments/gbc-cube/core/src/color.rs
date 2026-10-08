//! Colour: CGB palette RAM is BGR555; the panels (and the emulator's frame)
//! are RGB565.

/// BGR555 → RGB565 exactly as walnut-cgb converts it for its frame, so the
/// world renderer and the emulator agree pixel for pixel.
pub const fn bgr555_to_rgb565(c: u16) -> u16 {
    let r = c & 0x1F;
    let g = (c >> 5) & 0x1F;
    let b = (c >> 10) & 0x1F;
    let g = (g << 1) | (g >> 4);
    (r << 11) | (g << 5) | b
}

/// The 64 colours of CGB palette RAM as RGB565: background palettes 0–7
/// (indexes 0–31) then object palettes (32–63).
pub fn palette_from_ram(bg: &[u8], obj: &[u8]) -> [u16; 64] {
    let mut out = [0u16; 64];
    for i in 0..32 {
        out[i] = bgr555_to_rgb565(u16::from_le_bytes([bg[2 * i], bg[2 * i + 1]]));
        out[32 + i] = bgr555_to_rgb565(u16::from_le_bytes([obj[2 * i], obj[2 * i + 1]]));
    }
    out
}

/// Dim an RGB565 colour to `level`/255.
#[inline]
pub fn dim(c: u16, level: u8) -> u16 {
    if level == 255 {
        return c;
    }
    // x·l/255, rounded, without a divide: ×257 / 65536 is within a
    // rounding step of ÷255 over 0–255.
    let l = level as u32 * 257;
    let r = ((c >> 11) as u32 * l + 0x8000) >> 16;
    let g = (((c >> 5) & 0x3F) as u32 * l + 0x8000) >> 16;
    let b = ((c & 0x1F) as u32 * l + 0x8000) >> 16;
    ((r << 11) | (g << 5) | b) as u16
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_primaries() {
        assert_eq!(bgr555_to_rgb565(0x7FFF), 0xFFFF);
        assert_eq!(bgr555_to_rgb565(0x001F), 0xF800); // red is the low field
        assert_eq!(bgr555_to_rgb565(0x7C00), 0x001F);
        assert_eq!(dim(0xFFFF, 0), 0);
        assert_eq!(dim(0xFFFF, 255), 0xFFFF);
    }
}
