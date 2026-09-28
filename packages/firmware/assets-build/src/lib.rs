//! Builds the QSPI asset pack (see `smokebomb_shared::assets`).
//!
//! Runs on the host: the simulator HAL calls it from its build script, and
//! the `smokebomb-pack` binary writes a pack file for flashing. It
//!
//! - rasterizes Space Grotesk at every canvas size the screens use, with pair
//!   kerning measured by shaping each pair with HarfBuzz (rustybuzz), as the
//!   browser does for canvas text;
//! - renders the placeholder smoke clips (to be replaced by particle
//!   sprites, SIM_SPEC decision H1);
//! - lays both out as an SMKB v2 pack.

use std::io::Read;

use smokebomb_shared::assets::*;
use smokebomb_shared::types::FACE_COUNT;

/// Space Grotesk Bold, the fontsource build the golden frames were captured
/// with (SIL Open Font License, see `assets/fonts/OFL.txt`).
pub const SPACE_GROTESK_BOLD_WOFF: &[u8] =
    include_bytes!("../../assets/fonts/space-grotesk-latin-700-normal.woff");

/// Panel pixels per mockup canvas pixel.
pub const K: f32 = 96.0 / 166.0;

/// Every canvas size a screen may ask for: `fitPx` can produce any integer
/// up to its maximum, and the largest fixed size is the 94 px result number.
pub const CANVAS_SIZES: std::ops::RangeInclusive<u16> = 8..=94;

/// Characters the screens draw. ▲ ▼ ← → • are drawn as shapes (Space
/// Grotesk has no ▲ ▼; the mockup falls back to a system font for them).
pub fn charset() -> Vec<char> {
    let mut c: Vec<char> = (0x20u8..=0x7e).map(char::from).collect();
    c.extend(['×', '·', '…', '°']);
    c
}

// ---------- fonts ----------

/// Convert a WOFF 1.0 file to a plain sfnt (TTF/OTF).
pub fn woff_to_sfnt(woff: &[u8]) -> Result<Vec<u8>, String> {
    let be16 = |i: usize| u16::from_be_bytes([woff[i], woff[i + 1]]);
    let be32 = |i: usize| u32::from_be_bytes([woff[i], woff[i + 1], woff[i + 2], woff[i + 3]]);
    if woff.len() < 44 || &woff[0..4] != b"wOFF" {
        return Err("not a WOFF 1.0 file".into());
    }
    let flavor = be32(4);
    let num_tables = be16(12) as usize;
    struct Table {
        tag: [u8; 4],
        checksum: u32,
        data: Vec<u8>,
    }
    let mut tables = Vec::with_capacity(num_tables);
    for t in 0..num_tables {
        let e = 44 + t * 20;
        let (offset, comp, orig) = (be32(e + 4) as usize, be32(e + 8) as usize, be32(e + 12) as usize);
        let raw = woff.get(offset..offset + comp).ok_or("table out of range")?;
        let data = if comp < orig {
            let mut out = Vec::with_capacity(orig);
            flate2::read::ZlibDecoder::new(raw)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            out
        } else {
            raw.to_vec()
        };
        tables.push(Table {
            tag: woff[e..e + 4].try_into().unwrap(),
            checksum: be32(e + 16),
            data,
        });
    }
    tables.sort_by_key(|t| t.tag);

    let mut out = Vec::new();
    let pow2 = 1u16 << (15 - (num_tables as u16).leading_zeros());
    out.extend(flavor.to_be_bytes());
    out.extend((num_tables as u16).to_be_bytes());
    out.extend((pow2 * 16).to_be_bytes());
    out.extend((pow2.trailing_zeros() as u16).to_be_bytes());
    out.extend((num_tables as u16 * 16 - pow2 * 16).to_be_bytes());
    let mut offset = 12 + 16 * num_tables;
    for t in &tables {
        out.extend(t.tag);
        out.extend(t.checksum.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((t.data.len() as u32).to_be_bytes());
        offset += t.data.len().div_ceil(4) * 4;
    }
    for t in &tables {
        out.extend(&t.data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Ok(out)
}

/// Kerning in font units for every pair in `chars`, measured by shaping.
fn kerning_pairs(sfnt: &[u8], chars: &[char]) -> Vec<(char, char, i32)> {
    let face = rustybuzz::Face::from_slice(sfnt, 0).expect("font parses");
    let advance = |c: char| -> Option<i32> {
        let mut b = rustybuzz::UnicodeBuffer::new();
        b.push_str(&c.to_string());
        let out = rustybuzz::shape(&face, &[], b);
        out.glyph_positions().first().map(|p| p.x_advance)
    };
    let singles: Vec<Option<i32>> = chars.iter().map(|&c| advance(c)).collect();
    let mut pairs = Vec::new();
    for (i, &l) in chars.iter().enumerate() {
        let Some(al) = singles[i] else { continue };
        for &r in chars {
            let mut b = rustybuzz::UnicodeBuffer::new();
            b.push_str(&format!("{l}{r}"));
            let out = rustybuzz::shape(&face, &[], b);
            let pos = out.glyph_positions();
            if pos.len() == 2 {
                let k = pos[0].x_advance - al;
                if k != 0 {
                    pairs.push((l, r, k));
                }
            }
        }
    }
    pairs
}

/// One font section per canvas size: header, glyph table, kerning, bitmaps.
pub fn font_sections(sfnt: &[u8], weight: u16) -> Vec<(u16, Vec<u8>)> {
    let font = fontdue::Font::from_bytes(sfnt, fontdue::FontSettings::default()).expect("font parses");
    let units_per_em = font.units_per_em();
    let mut chars: Vec<char> = charset()
        .into_iter()
        .filter(|&c| font.lookup_glyph_index(c) != 0 || c == ' ')
        .collect();
    chars.sort();
    let kerns = kerning_pairs(sfnt, &chars);

    CANVAS_SIZES
        .map(|canvas_px| {
            let px = canvas_px as f32 * K;
            let q6 = |v: f32| (v * Q6).round() as i32;
            let line = font.horizontal_line_metrics(px).expect("horizontal metrics");

            let mut glyphs = Vec::new();
            let mut bitmaps = Vec::new();
            for &c in &chars {
                let (m, bitmap) = font.rasterize(c, px);
                glyphs.push((c, m, bitmaps.len()));
                bitmaps.extend(bitmap);
            }
            let mut kern_table: Vec<(u16, u16, i16)> = kerns
                .iter()
                .filter_map(|&(l, r, units)| {
                    let adj = q6(units as f32 * px / units_per_em);
                    (adj != 0).then_some((l as u16, r as u16, adj as i16))
                })
                .collect();
            kern_table.sort();

            let glyph_table_end = FONT_HEADER_LEN + glyphs.len() * GLYPH_ENTRY_LEN;
            let kern_offset = glyph_table_end;
            let bitmaps_start = kern_offset + kern_table.len() * KERN_ENTRY_LEN;

            let mut out = Vec::new();
            out.extend(
                FontHeader {
                    weight,
                    canvas_px,
                    glyph_count: glyphs.len() as u16,
                    kern_count: kern_table.len() as u16,
                    ascent_q6: q6(line.ascent) as i16,
                    descent_q6: q6(line.descent) as i16,
                    kern_offset: kern_offset as u32,
                }
                .encode(),
            );
            for (c, m, start) in &glyphs {
                out.extend(
                    GlyphEntry {
                        codepoint: *c as u32,
                        advance_q6: q6(m.advance_width) as u16,
                        width: u8::try_from(m.width).expect("glyph fits 255 px"),
                        height: u8::try_from(m.height).expect("glyph fits 255 px"),
                        x_min: m.xmin as i16,
                        y_min: m.ymin as i16,
                        bitmap_offset: (bitmaps_start + start) as u32,
                    }
                    .encode(),
                );
            }
            for (l, r, adj) in kern_table {
                out.extend(
                    KernEntry {
                        left: l,
                        right: r,
                        adjust_q6: adj,
                    }
                    .encode(),
                );
            }
            out.extend(bitmaps);
            (canvas_px, out)
        })
        .collect()
}

// ---------- placeholder clips ----------

pub struct Clip {
    pub id: ClipId,
    pub fps: u8,
    pub looping: bool,
    /// Six packed faces per frame.
    pub frames: Vec<Vec<u8>>,
}

/// Cheap procedural rings standing in for smoke until the particle system
/// lands (SIM_SPEC H1).
pub fn placeholder_clips() -> Vec<Clip> {
    [
        (ClipId::SmokeIdle, 30u16, true),
        (ClipId::SmokeShake, 20, true),
        (ClipId::SmokeThrow, 15, true),
        (ClipId::MaxBurst, 30, false),
        (ClipId::Dud, 15, false),
    ]
    .into_iter()
    .map(|(id, frames, looping)| Clip {
        id,
        fps: 30,
        looping,
        frames: (0..frames)
            .map(|f| {
                (0..FACE_COUNT)
                    .flat_map(|face| ring_frame(id, f, frames, face))
                    .collect()
            })
            .collect(),
    })
    .collect()
}

/// Expanding soft ring, phase-shifted per face so the cube shimmers.
fn ring_frame(clip: ClipId, frame: u16, frames: u16, face: usize) -> Vec<u8> {
    let t = (frame as f32 / frames as f32 + face as f32 / 6.0) % 1.0;
    let radius = 8.0 + t * 48.0;
    let width = match clip {
        ClipId::MaxBurst => 10.0,
        ClipId::SmokeShake => 4.0,
        _ => 7.0,
    };
    let peak = if clip == ClipId::Dud {
        3.0
    } else {
        9.0 * (1.0 - t * 0.6)
    };
    let mut buf = vec![0u8; FRAME_BYTES];
    for y in 0..PANEL_HEIGHT {
        for x in 0..PANEL_WIDTH {
            let (dx, dy) = (x as f32 - 47.5, y as f32 - 47.5);
            let d = (dx * dx + dy * dy).sqrt();
            let level = (peak * (1.0 - ((d - radius).abs() / width)).max(0.0)) as u8;
            let i = y * PANEL_WIDTH + x;
            buf[i / 2] |= if i % 2 == 0 { level << 4 } else { level & 0x0f };
        }
    }
    buf
}

// ---------- pack ----------

pub enum Section {
    Clips(Vec<Clip>),
    /// `(canvas_px, bytes)` from [`font_sections`].
    Font(u16, Vec<u8>),
}

pub fn build_pack(sections: Vec<Section>) -> Vec<u8> {
    let table_len = HEADER_LEN + sections.len() * SECTION_ENTRY_LEN;
    let mut body: Vec<u8> = Vec::new();
    let mut entries = Vec::new();
    for s in sections {
        let offset = (table_len + body.len()) as u32;
        let (kind, id, bytes) = match s {
            Section::Font(px, bytes) => (SectionKind::Font, px, bytes),
            Section::Clips(clips) => {
                // Clip frames follow the clip table; offsets are absolute.
                let header = CLIPS_HEADER_LEN + clips.len() * CLIP_ENTRY_LEN;
                let mut table = Vec::new();
                table.extend((clips.len() as u16).to_le_bytes());
                table.extend([0u8; 2]);
                let mut data: Vec<u8> = Vec::new();
                for c in &clips {
                    table.extend(
                        ClipEntry {
                            id: c.id as u16,
                            frame_count: c.frames.len() as u16,
                            fps: c.fps,
                            flags: if c.looping { ClipEntry::FLAG_LOOP } else { 0 },
                            offset: offset + (header + data.len()) as u32,
                        }
                        .encode(),
                    );
                    for f in &c.frames {
                        assert_eq!(f.len(), CUBE_FRAME_BYTES);
                        data.extend(f);
                    }
                }
                table.extend(data);
                (SectionKind::Clips, 0, table)
            }
        };
        entries.push(SectionEntry {
            kind: kind as u16,
            id,
            offset,
            len: bytes.len() as u32,
        });
        body.extend(bytes);
        body.resize(body.len().div_ceil(4) * 4, 0);
    }
    let mut out = Vec::with_capacity(table_len + body.len());
    out.extend(
        PackHeader {
            version: PACK_VERSION,
            section_count: entries.len() as u16,
            total_len: (table_len + body.len()) as u32,
        }
        .encode(),
    );
    for e in &entries {
        out.extend(e.encode());
    }
    out.extend(body);
    out
}

/// The standard pack: Space Grotesk Bold at every size, plus placeholder clips.
pub fn standard_pack() -> Vec<u8> {
    let sfnt = woff_to_sfnt(SPACE_GROTESK_BOLD_WOFF).expect("bundled font converts");
    let mut sections = vec![Section::Clips(placeholder_clips())];
    sections.extend(
        font_sections(&sfnt, 700)
            .into_iter()
            .map(|(px, bytes)| Section::Font(px, bytes)),
    );
    build_pack(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_pack_has_fonts_and_clips() {
        let pack = standard_pack();
        let header = PackHeader::decode(pack[..HEADER_LEN].try_into().unwrap()).unwrap();
        assert_eq!(header.version, PACK_VERSION);
        assert_eq!(header.total_len as usize, pack.len());
        assert_eq!(header.section_count as usize, 1 + CANVAS_SIZES.count());
        assert!(pack.len() < QSPI_CAPACITY as usize);
    }

    #[test]
    fn space_grotesk_has_kerning() {
        let sfnt = woff_to_sfnt(SPACE_GROTESK_BOLD_WOFF).unwrap();
        let chars = charset();
        assert!(!kerning_pairs(&sfnt, &chars).is_empty());
    }
}
