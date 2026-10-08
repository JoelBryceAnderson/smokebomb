//! PNG output for screenshots: RGB565 in, scaled up, RGB out.

use std::path::Path;

use anyhow::Result;

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 3]>,
}

pub fn rgb(c: u16) -> [u8; 3] {
    let r = (c >> 11) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: [u8; 3]) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![bg; w * h],
        }
    }

    pub fn set(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }

    /// A `scale`×`scale` block for source pixel (`x`, `y`) at `origin`.
    pub fn put(&mut self, origin: (i32, i32), scale: i32, x: i32, y: i32, c: [u8; 3]) {
        for j in 0..scale {
            for i in 0..scale {
                self.set(origin.0 + x * scale + i, origin.1 + y * scale + j, c);
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header()?;
        let flat: Vec<u8> = self.px.iter().flatten().copied().collect();
        w.write_image_data(&flat)?;
        Ok(())
    }
}
