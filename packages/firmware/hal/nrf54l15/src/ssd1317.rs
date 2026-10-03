//! SSD1317 OLED driver: six 96x96 panels on one SPI bus, sharing clock,
//! data, D/C and reset, each with its own chip-select.
//!
//! The SSD1317 is Solomon's 128x96 controller, with the SSD1306 family's
//! command set: one bit a pixel, in 12 pages of 8 rows, each byte a column
//! of 8 pixels, least significant bit on top. A 96-column panel is wired to
//! segments 16-111, so its columns start at [`COLUMN_OFFSET`].
//!
//! The firmware draws 4bpp frames (16 levels); [`pack_page`] reduces them to
//! the panel's one bit at [`LIT_LEVEL`]. That keeps the controller lit and
//! testable while how the faces show grey is still open.
//!
//! The commands and values follow u8g2's SSD1317 96x96 driver
//! (`u8x8_d_ssd1317.c`), the one known to light this panel, except that
//! this one uses page addressing (u8g2 sets horizontal, then addresses
//! pages anyway) and leaves out u8g2's SSD1306 charge-pump command: the
//! panels get their 12 V from outside. Check them against the SSD1317
//! datasheet on the bench: the remap (`0xA0`/`0xC8`) sets which way up a
//! face is.
//!
//! The driver only builds byte streams; [`PanelBus`] moves them. On the
//! board that is the Zephyr shim, and in the tests a recording mock, so the
//! exact bytes each panel gets are checked on the host.

use smokebomb_hal::{FrameBytes, HalResult, PANEL_HEIGHT, PANEL_WIDTH};

/// First controller column the panel's pixels are wired to.
pub const COLUMN_OFFSET: u8 = 16;
/// Pages of 8 rows on a panel.
pub const PAGES: usize = PANEL_HEIGHT / 8;
/// Lowest 4bpp level that lights a pixel.
pub const LIT_LEVEL: u8 = 8;
/// How long reset is held low, and how long to wait after it, in ms.
pub const RESET_MS: u32 = 10;

/// What the D/C line says about the bytes on the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dc {
    /// Commands and their parameters (D/C low).
    Command,
    /// Display RAM (D/C high).
    Data,
}

/// The bus the panels share.
pub trait PanelBus {
    /// Select `panel` (0-5), set D/C and write `bytes`, then deselect.
    fn write(&mut self, panel: u8, dc: Dc, bytes: &[u8]) -> HalResult<()>;
    /// Drive the shared reset line (active low on the wire).
    fn set_reset(&mut self, asserted: bool) -> HalResult<()>;
    fn delay_ms(&mut self, ms: u32);
}

/// Display off, then everything the panel needs to show its RAM. The panel
/// stays off until [`Ssd1317::set_on`].
pub const INIT: &[u8] = &[
    0xAE, // display off
    0xD5, 0xD1, // clock: divide by 2, oscillator 0xD
    0xA8, 0x5F, // multiplex ratio: 96 rows
    0xD3, 0x00, // display offset 0
    0xA2, 0x00, // display start line 0
    0x20, 0x02, // page addressing
    0xA0, // segment remap: column 0 on SEG0
    0xC8, // COM scan: reversed
    0xDA, 0x12, // COM pins: alternative, no left/right remap
    0x81, 0x9F, // contrast
    0xD9, 0xF1, // pre-charge: phase 1 = 1, phase 2 = 15 clocks
    0xDB, 0xFF, // VCOMH deselect level
    0x2E, // scrolling off
    0xA4, // show RAM (not all-on)
    0xA6, // normal, not inverted
];

pub const DISPLAY_OFF: u8 = 0xAE;
pub const DISPLAY_ON: u8 = 0xAF;
pub const CONTRAST: u8 = 0x81;

/// Commands that point the RAM at the start of `page`, column 0 of the panel.
pub fn page_address(page: u8) -> [u8; 3] {
    [0xB0 | page, COLUMN_OFFSET & 0x0F, 0x10 | (COLUMN_OFFSET >> 4)]
}

/// One page (8 rows) of a 4bpp frame as the panel's bytes: a byte a column,
/// row `8 * page` in bit 0.
pub fn pack_page(frame: &FrameBytes, page: usize, out: &mut [u8; PANEL_WIDTH]) {
    for (x, byte) in out.iter_mut().enumerate() {
        let mut b = 0;
        for bit in 0..8 {
            let i = (page * 8 + bit) * PANEL_WIDTH + x;
            let packed = frame[i / 2];
            let level = if i % 2 == 0 { packed >> 4 } else { packed & 0x0F };
            if level >= LIT_LEVEL {
                b |= 1 << bit;
            }
        }
        *byte = b;
    }
}

pub struct Ssd1317<B> {
    bus: B,
    page: [u8; PANEL_WIDTH],
}

impl<B: PanelBus> Ssd1317<B> {
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            page: [0; PANEL_WIDTH],
        }
    }

    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    /// Pulse the shared reset line: every panel forgets its setup.
    pub fn reset(&mut self) -> HalResult<()> {
        self.bus.set_reset(true)?;
        self.bus.delay_ms(RESET_MS);
        self.bus.set_reset(false)?;
        self.bus.delay_ms(RESET_MS);
        Ok(())
    }

    /// Set up one panel after reset. It stays off.
    pub fn init(&mut self, panel: u8) -> HalResult<()> {
        self.bus.write(panel, Dc::Command, INIT)
    }

    pub fn set_on(&mut self, panel: u8, on: bool) -> HalResult<()> {
        let cmd = if on { DISPLAY_ON } else { DISPLAY_OFF };
        self.bus.write(panel, Dc::Command, &[cmd])
    }

    pub fn set_contrast(&mut self, panel: u8, level: u8) -> HalResult<()> {
        self.bus.write(panel, Dc::Command, &[CONTRAST, level])
    }

    /// Send a whole frame to one panel, a page at a time.
    pub fn write_frame(&mut self, panel: u8, frame: &FrameBytes) -> HalResult<()> {
        for page in 0..PAGES {
            pack_page(frame, page, &mut self.page);
            self.bus.write(panel, Dc::Command, &page_address(page as u8))?;
            self.bus.write(panel, Dc::Data, &self.page)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::*;
    use smokebomb_hal::FRAME_BYTES;

    #[derive(Debug, PartialEq, Eq)]
    enum Op {
        Write(u8, Dc, Vec<u8>),
        Reset(bool),
        Delay(u32),
    }

    #[derive(Default)]
    struct MockBus(Vec<Op>);

    impl PanelBus for MockBus {
        fn write(&mut self, panel: u8, dc: Dc, bytes: &[u8]) -> HalResult<()> {
            self.0.push(Op::Write(panel, dc, bytes.to_vec()));
            Ok(())
        }
        fn set_reset(&mut self, asserted: bool) -> HalResult<()> {
            self.0.push(Op::Reset(asserted));
            Ok(())
        }
        fn delay_ms(&mut self, ms: u32) {
            self.0.push(Op::Delay(ms));
        }
    }

    fn ops(f: impl FnOnce(&mut Ssd1317<MockBus>)) -> Vec<Op> {
        let mut d = Ssd1317::new(MockBus::default());
        f(&mut d);
        d.bus.0
    }

    #[test]
    fn reset_holds_the_line_low_then_waits() {
        let ops = ops(|d| d.reset().unwrap());
        assert_eq!(
            ops,
            vec![
                Op::Reset(true),
                Op::Delay(RESET_MS),
                Op::Reset(false),
                Op::Delay(RESET_MS)
            ]
        );
    }

    #[test]
    fn init_is_one_command_burst_that_leaves_the_panel_off() {
        let ops = ops(|d| d.init(3).unwrap());
        assert_eq!(ops, vec![Op::Write(3, Dc::Command, INIT.to_vec())]);
        assert_eq!(INIT[0], DISPLAY_OFF);
        assert!(!INIT.contains(&DISPLAY_ON));
        // 96 rows multiplexed.
        let mux = INIT.iter().position(|&b| b == 0xA8).unwrap();
        assert_eq!(INIT[mux + 1] as usize + 1, PANEL_HEIGHT);
    }

    #[test]
    fn on_off_and_contrast() {
        let ops = ops(|d| {
            d.set_on(0, true).unwrap();
            d.set_on(5, false).unwrap();
            d.set_contrast(2, 0x40).unwrap();
        });
        assert_eq!(
            ops,
            vec![
                Op::Write(0, Dc::Command, vec![0xAF]),
                Op::Write(5, Dc::Command, vec![0xAE]),
                Op::Write(2, Dc::Command, vec![0x81, 0x40]),
            ]
        );
    }

    #[test]
    fn page_address_starts_at_the_panels_first_column() {
        assert_eq!(page_address(0), [0xB0, 0x00, 0x11]);
        assert_eq!(page_address(11), [0xBB, 0x00, 0x11]);
    }

    #[test]
    fn a_frame_is_twelve_pages_of_96_bytes() {
        let frame = [0xFF; FRAME_BYTES];
        let ops = ops(|d| d.write_frame(4, &frame).unwrap());
        assert_eq!(ops.len(), 2 * PAGES);
        for (page, pair) in ops.chunks(2).enumerate() {
            assert_eq!(
                pair[0],
                Op::Write(4, Dc::Command, page_address(page as u8).to_vec())
            );
            assert_eq!(pair[1], Op::Write(4, Dc::Data, vec![0xFF; PANEL_WIDTH]));
        }
    }

    /// Set one pixel of a 4bpp frame to `level`.
    fn set(frame: &mut FrameBytes, x: usize, y: usize, level: u8) {
        let i = y * PANEL_WIDTH + x;
        let b = &mut frame[i / 2];
        *b = if i % 2 == 0 {
            (*b & 0x0F) | (level << 4)
        } else {
            (*b & 0xF0) | level
        };
    }

    #[test]
    fn pixels_land_in_their_column_byte_top_row_in_bit_0() {
        let mut frame = [0; FRAME_BYTES];
        set(&mut frame, 0, 0, 15); // top left
        set(&mut frame, 1, 7, 15); // odd column, bottom of page 0
        set(&mut frame, 95, 95, 15); // bottom right: page 11, bit 7
        set(&mut frame, 2, 3, LIT_LEVEL - 1); // just too dim to light
        set(&mut frame, 3, 3, LIT_LEVEL); // just bright enough
        let mut out = [0; PANEL_WIDTH];
        pack_page(&frame, 0, &mut out);
        assert_eq!(out[0], 0b0000_0001);
        assert_eq!(out[1], 0b1000_0000);
        assert_eq!(out[2], 0);
        assert_eq!(out[3], 0b0000_1000);
        assert!(out[4..].iter().all(|&b| b == 0));
        pack_page(&frame, 11, &mut out);
        assert_eq!(out[95], 0b1000_0000);
        assert!(out[..95].iter().all(|&b| b == 0));
    }
}
