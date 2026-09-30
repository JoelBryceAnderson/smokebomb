//! Builds the QSPI asset pack (see `smokebomb_shared::assets`).
//!
//! Runs on the host: the simulator HAL calls it from its build script, and
//! the `smokebomb-pack` binary writes a pack file for flashing. It
//!
//! - rasterizes Space Grotesk at every canvas size the screens use, with pair
//!   kerning measured by shaping each pair with HarfBuzz (rustybuzz), as the
//!   browser does for canvas text;
//! - rasterizes Pacifico, the Sugarcube wordmark's retro script, at the few
//!   sizes the wordmark is drawn at;
//! - bakes the smoke sprites the particle system stamps (SIM_SPEC D1, D3);
//! - lays both out as an SMKB v3 pack.

use std::io::Read;

use smokebomb_shared::assets::*;

/// Space Grotesk Bold, the fontsource build the mockup used (SIL Open Font License, see `assets/fonts/OFL.txt`).
pub const SPACE_GROTESK_BOLD_WOFF: &[u8] =
    include_bytes!("../../assets/fonts/space-grotesk-latin-700-normal.woff");

/// Pacifico, the retro script the Sugarcube wordmark is set in (fontsource
/// build, SIL Open Font License, see `assets/fonts/OFL-Pacifico.txt`).
pub const PACIFICO_WOFF: &[u8] = include_bytes!("../../assets/fonts/pacifico-latin-400-normal.woff");

/// Panel pixels per mockup canvas pixel.
pub const K: f32 = 96.0 / 166.0;

/// Every canvas size a screen may ask for: `fitPx` can produce any integer
/// up to its maximum, and the largest fixed size is the 94 px result number.
pub const CANVAS_SIZES: std::ops::RangeInclusive<u16> = 8..=94;

/// Canvas sizes of the script face: the wordmark on the boot screen.
pub const SCRIPT_SIZES: std::ops::RangeInclusive<u16> = 30..=36;

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
pub fn font_sections(sfnt: &[u8], weight: u16, sizes: std::ops::RangeInclusive<u16>) -> Vec<(u16, Vec<u8>)> {
    let font = fontdue::Font::from_bytes(sfnt, fontdue::FontSettings::default()).expect("font parses");
    let units_per_em = font.units_per_em();
    let mut chars: Vec<char> = charset()
        .into_iter()
        .filter(|&c| font.lookup_glyph_index(c) != 0 || c == ' ')
        .collect();
    chars.sort();
    let kerns = kerning_pairs(sfnt, &chars);

    sizes
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

// ---------- sprites ----------

/// The mockup's sprites: a radial gradient at full opacity in the centre,
/// 45% at 0.45 of the radius and clear at the edge, in each kind's colour.
/// The grey value is the colour's brightest channel, as for screen colours.
pub fn sprites() -> Vec<SpriteEntry> {
    let alpha = |r: f32| {
        if r <= 0.45 {
            1.0 - (1.0 - 0.45) * r / 0.45
        } else {
            0.45 * (1.0 - r) / (1.0 - 0.45)
        }
    };
    let profile: [u8; SPRITE_PROFILE_LEN] =
        std::array::from_fn(|i| (alpha(i as f32 / (SPRITE_PROFILE_LEN - 1) as f32) * 255.0).round() as u8);
    [
        (SpriteKind::Smoke, [214u8, 219, 226]),
        (SpriteKind::Ember, [246, 247, 249]),
        (SpriteKind::Gold, [255, 255, 255]),
        (SpriteKind::Fizzle, [120, 122, 126]),
    ]
    .into_iter()
    .map(|(kind, rgb)| SpriteEntry {
        kind: kind as u8,
        value: rgb.into_iter().max().unwrap_or(0),
        profile,
    })
    .collect()
}

// ---------- pack ----------

pub enum Section {
    Sprites(Vec<SpriteEntry>),
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
            Section::Sprites(sprites) => {
                let mut table = Vec::new();
                table.extend((sprites.len() as u16).to_le_bytes());
                table.extend([0u8; 2]);
                for sp in &sprites {
                    table.extend(sp.encode());
                }
                (SectionKind::Sprites, 0, table)
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

/// The standard pack: Space Grotesk Bold at every size, the wordmark
/// script, plus the smoke sprites.
pub fn standard_pack() -> Vec<u8> {
    let sfnt = woff_to_sfnt(SPACE_GROTESK_BOLD_WOFF).expect("bundled font converts");
    let script = woff_to_sfnt(PACIFICO_WOFF).expect("bundled script converts");
    let mut sections = vec![Section::Sprites(sprites())];
    sections.extend(
        font_sections(&sfnt, 700, CANVAS_SIZES)
            .into_iter()
            .chain(font_sections(&script, SCRIPT_WEIGHT, SCRIPT_SIZES))
            .map(|(px, bytes)| Section::Font(px, bytes)),
    );
    build_pack(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_pack_has_fonts_and_sprites() {
        let pack = standard_pack();
        let header = PackHeader::decode(pack[..HEADER_LEN].try_into().unwrap()).unwrap();
        assert_eq!(header.version, PACK_VERSION);
        assert_eq!(header.total_len as usize, pack.len());
        assert_eq!(
            header.section_count as usize,
            1 + CANVAS_SIZES.count() + SCRIPT_SIZES.count()
        );
        assert!(pack.len() < QSPI_CAPACITY as usize);
    }

    #[test]
    fn sprites_match_the_mockup_gradient() {
        let smoke = sprites()[SpriteKind::Smoke as usize];
        assert_eq!(smoke.value, 226);
        assert_eq!(smoke.profile[0], 255);
        // 0.45 of the radius is sample 28.8: 45% there.
        assert!(
            (smoke.profile[29] as i32 - 114).abs() <= 2,
            "{}",
            smoke.profile[29]
        );
        assert_eq!(smoke.profile[SPRITE_PROFILE_LEN - 1], 0);
    }

    #[test]
    fn space_grotesk_has_kerning() {
        let sfnt = woff_to_sfnt(SPACE_GROTESK_BOLD_WOFF).unwrap();
        let chars = charset();
        assert!(!kerning_pairs(&sfnt, &chars).is_empty());
    }
}
