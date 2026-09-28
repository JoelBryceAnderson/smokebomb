//! Reading the QSPI asset pack's section table (see
//! [`smokebomb_shared::assets`] for the layout).

use heapless::Vec;
use smokebomb_hal::AssetStore;
use smokebomb_shared::assets::{
    PackHeader, SectionEntry, SectionKind, HEADER_LEN, PACK_VERSION, SECTION_ENTRY_LEN,
};

/// Enough for one clip table plus a font per canvas size (8–94) and weight.
pub const MAX_SECTIONS: usize = 192;

#[derive(Default)]
pub struct PackIndex {
    sections: Vec<SectionEntry, MAX_SECTIONS>,
}

impl PackIndex {
    /// Read the section table. A missing, old or corrupt pack gives an empty
    /// index: the die still works, it just draws nothing from flash.
    pub fn load<A: AssetStore>(assets: &mut A) -> Self {
        let mut index = Self::default();
        let mut hdr = [0u8; HEADER_LEN];
        if assets.read(0, &mut hdr).is_err() {
            return index;
        }
        let Some(h) = PackHeader::decode(&hdr).filter(|h| h.version == PACK_VERSION) else {
            return index;
        };
        for i in 0..(h.section_count as usize).min(MAX_SECTIONS) {
            let mut e = [0u8; SECTION_ENTRY_LEN];
            let off = (HEADER_LEN + i * SECTION_ENTRY_LEN) as u32;
            if assets.read(off, &mut e).is_err() {
                break;
            }
            let _ = index.sections.push(SectionEntry::decode(&e));
        }
        index
    }

    pub fn sections(&self, kind: SectionKind) -> impl Iterator<Item = &SectionEntry> {
        self.sections.iter().filter(move |s| s.kind == kind as u16)
    }
}
