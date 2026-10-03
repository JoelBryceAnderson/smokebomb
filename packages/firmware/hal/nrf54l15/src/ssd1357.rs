//! SSD1357 driver for the 30 mm die's six 0.6" 64×64 RGB PMOLEDs
//! (Newhaven NHD-0.6-6464G class), 4-wire SPI.
//!
//! **Sources.** Every command byte and its parameters are from the Solomon
//! Systech *SSD1357 Advance Information, Rev 1.0, Nov 2016*, and its appendix
//! *SSD1357 Command Table and Command Description* (cited below as "DS" with
//! page and section, and "CT" with the appendix page). Values marked
//! **MODULE TODO** depend on how the NHD-0.6-6464G wires its 64×64 glass to
//! the controller's 128×128 (which SEG/COM lines, remap, VCC, tuning); they
//! need that module's datasheet and are placeholders, not guesses dressed up
//! as facts: most are the controller's reset values.
//!
//! **Structure.** [`Ssd1357`] holds the panel logic over a [`PanelBus`]:
//! chip-select per face, D/C#, reset, the VCC rail and delays. The logic is
//! complete and tested on the host by recording the bus traffic.
//! [`ZephyrPanels`], the bus on the board, is a stub until the SPIM wiring
//! exists (see the crate docs).

use smokebomb_hal::{Display, Face, HalError, HalResult, Region, Rgb64, FACE_COUNT};

// ---------- commands (CT Table 1-1, pp. 1–6) ----------

/// Set Column Address: start, end (0–127). CT p. 1, §2.1.
pub const SET_COLUMN: u8 = 0x15;
/// Set Row Address: start, end (0–127). CT p. 1, §2.2.
pub const SET_ROW: u8 = 0x75;
/// Write RAM: the data bytes that follow go to the window. CT p. 1, §2.3.
pub const WRITE_RAM: u8 = 0x5C;
/// Set Re-map / Color Depth: A, then B (= 00h). CT p. 1, §2.5.
pub const REMAP: u8 = 0xA0;
/// Set Display Start Line (0–127). CT p. 2, §2.6.
pub const START_LINE: u8 = 0xA1;
/// Set Display Offset (0–127). CT p. 2, §2.7.
pub const DISPLAY_OFFSET: u8 = 0xA2;
/// Display mode: A4h all off, A5h all on, A6h normal, A7h inverse. CT p. 2.
pub const DISPLAY_NORMAL: u8 = 0xA6;
pub const DISPLAY_ALL_OFF: u8 = 0xA4;
/// Sleep mode on (display off). CT p. 2.
pub const SLEEP_ON: u8 = 0xAE;
/// Sleep mode off (display on). CT p. 2.
pub const SLEEP_OFF: u8 = 0xAF;
/// Phase 1 (A[3:0]) and phase 2 (A[7:4]) periods. CT p. 2.
pub const PHASE_PERIOD: u8 = 0xB1;
/// Front clock divider A[3:0] and oscillator frequency A[7:4]. CT p. 3.
pub const CLOCK: u8 = 0xB3;
/// Second pre-charge period A[3:0]. CT p. 3.
pub const SECOND_PRECHARGE: u8 = 0xB6;
/// Use the built-in linear gray-scale LUT. CT p. 4.
pub const LINEAR_LUT: u8 = 0xB9;
/// Pre-charge voltage A[4:0]. CT p. 4.
pub const PRECHARGE_VOLTAGE: u8 = 0xBB;
/// COM deselect voltage (VCOMH) A[2:0]. CT p. 5.
pub const VCOMH: u8 = 0xBE;
/// Contrast current for colours A, B, C (three bytes). CT p. 5.
pub const CONTRAST: u8 = 0xC1;
/// Master contrast current A[3:0]: (A + 1)/16 of the output. CT p. 5.
pub const MASTER_CURRENT: u8 = 0xC7;
/// MUX ratio A[6:0] = rows − 1 (3–127). CT p. 5.
pub const MUX_RATIO: u8 = 0xCA;
/// Command lock: 12h unlocks, 16h locks. CT p. 6.
pub const COMMAND_LOCK: u8 = 0xFD;
pub const UNLOCK: u8 = 0x12;

// ---------- the module (MODULE TODO: NHD-0.6-6464G datasheet) ----------

/// The panel is 64×64 (NHD-0.6-6464G).
pub const SIDE: u8 = 64;
/// First RAM column the glass shows. **MODULE TODO**: which 64 of the 128
/// SEG triplets the module bonds. 0 is a placeholder.
pub const COLUMN_OFFSET: u8 = 0;
/// First RAM row the glass shows. **MODULE TODO**: which 64 of the 128 COM
/// lines. 0 is a placeholder.
pub const ROW_OFFSET: u8 = 0;
/// Re-map A byte. Bits from CT p. 1: A[7:6] = 01b 65k colour (needed: the
/// firmware sends RGB565, two bytes a pixel, DS Table 6-7); A[0] = 0
/// horizontal address increment (needed: rows go out left to right). The
/// rest is **MODULE TODO**: A[1] column remap, A[2] colour order (A B C or
/// C B A: which sub-pixel is red), A[4] COM scan direction, A[5] COM odd/even
/// split (reset 1). Placeholder: the reset value, 65k, split on.
pub const REMAP_A: u8 = 0b0110_0000;
/// MUX ratio for 64 rows: 64 − 1 (CT p. 5). **MODULE TODO**: confirm the
/// module drives 64 COM lines.
pub const MUX_64: u8 = SIDE - 1;
/// Phase periods. Reset value 84h (phase 1 = 8 DCLK, phase 2 = 16 DCLK, CT
/// p. 2). **MODULE TODO**: tune to the glass's capacitance.
pub const PHASE_A: u8 = 0x84;
/// Clock: divide by 1, oscillator level 2 (reset, CT p. 3). With K = 169
/// DCLK a row at reset (DS §6.3) and 64 MUX, frame rate = Fosc / (1 × 169 ×
/// 64) ≈ 194–231 Hz for Fosc 2.1–2.5 MHz (DS Table 9-1): above the 60 Hz the
/// firmware draws at.
pub const CLOCK_A: u8 = 0x20;
/// Second pre-charge: 8 DCLK (reset, CT p. 3). **MODULE TODO**.
pub const SECOND_PRECHARGE_A: u8 = 0x08;
/// Pre-charge voltage: 1Eh = 0.50 × VCC (reset, CT p. 4). **MODULE TODO**.
pub const PRECHARGE_A: u8 = 0x1E;
/// VCOMH: 05h = 0.82 × VCC (reset, CT p. 5). **MODULE TODO**.
pub const VCOMH_A: u8 = 0x05;
/// Contrast per colour at full brightness. Reset is 7Fh each (CT p. 5);
/// ISEG = contrast / 8 × IREF (DS §6.6). **MODULE TODO**: the module's
/// white balance and its current limit.
pub const CONTRAST_FULL: [u8; 3] = [0x7F, 0x7F, 0x7F];

// ---------- timing (DS §6.9, Figure 6-18; Table 9-1) ----------

/// After VDD is stable, before RES# goes low: at least 20 ms (t0).
pub const T0_VDD_TO_RESET_US: u32 = 20_000;
/// RES# low pulse: at least 3 µs (t1, tRES).
pub const T1_RESET_LOW_US: u32 = 3;
/// After RES# low, before VCC on: at least 3 µs (t2).
pub const T2_RESET_TO_VCC_US: u32 = 3;
/// After VDD is stable, before the first command: at least 300 ms.
pub const VDD_TO_COMMAND_US: u32 = 300_000;
/// SEG/COM turn on 200 ms after AFh (tAF).
pub const T_AF_US: u32 = 200_000;
/// Power off: AEh, VCC off, then VDD after tOFF (min 0, typ 100 ms).
pub const T_OFF_US: u32 = 100_000;
/// 4-wire SPI clock cycle ≥ 100 ns: SCLK ≤ 10 MHz (DS Table 9-4, p. 32).
pub const MAX_SCLK_HZ: u32 = 10_000_000;

/// What the driver needs from the board. One bus serves every face; the
/// face picks the chip-select line.
pub trait PanelBus {
    /// Send command bytes (D/C# low for the first, high for its
    /// parameters, as the 4-wire interface takes them; DS §6.1.3).
    fn command(&mut self, face: Face, cmd: u8, params: &[u8]) -> HalResult<()>;
    /// Send display data (D/C# high), after [`WRITE_RAM`].
    fn data(&mut self, face: Face, bytes: &[u8]) -> HalResult<()>;
    /// Drive every panel's RES# (shared line).
    fn reset(&mut self, low: bool) -> HalResult<()>;
    /// Switch the panels' VCC rail (8–18 V, DS §2).
    fn vcc(&mut self, on: bool) -> HalResult<()>;
    fn delay_us(&mut self, us: u32);
}

/// The six panels behind one [`PanelBus`].
pub struct Ssd1357<B: PanelBus> {
    pub bus: B,
    powered: bool,
}

/// The configuration after reset, in order (each a command and its
/// parameters). Display stays asleep; [`Ssd1357::power_on`] wakes it after
/// clearing RAM.
pub const INIT: &[(u8, &[u8])] = &[
    (COMMAND_LOCK, &[UNLOCK]),
    (SLEEP_ON, &[]),
    (CLOCK, &[CLOCK_A]),
    (MUX_RATIO, &[MUX_64]),
    (REMAP, &[REMAP_A, 0x00]),
    (START_LINE, &[0]),
    (DISPLAY_OFFSET, &[0]),
    (PHASE_PERIOD, &[PHASE_A]),
    (SECOND_PRECHARGE, &[SECOND_PRECHARGE_A]),
    (PRECHARGE_VOLTAGE, &[PRECHARGE_A]),
    (VCOMH, &[VCOMH_A]),
    (LINEAR_LUT, &[]),
    (CONTRAST, &CONTRAST_FULL),
    (MASTER_CURRENT, &[0x0F]),
    (DISPLAY_NORMAL, &[]),
];

impl<B: PanelBus> Ssd1357<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus, powered: false }
    }

    /// The power-on sequence of DS §6.9 for every panel, then configure,
    /// clear and wake them. Assumes VDD has just come up.
    pub fn power_on(&mut self) -> HalResult<()> {
        self.bus.delay_us(T0_VDD_TO_RESET_US);
        self.bus.reset(true)?;
        self.bus.delay_us(T1_RESET_LOW_US.max(T2_RESET_TO_VCC_US));
        self.bus.vcc(true)?;
        self.bus.reset(false)?;
        self.bus
            .delay_us(VDD_TO_COMMAND_US.saturating_sub(T0_VDD_TO_RESET_US));
        for face in Face::ALL {
            for &(cmd, params) in INIT {
                self.bus.command(face, cmd, params)?;
            }
            self.clear(face)?;
            self.bus.command(face, SLEEP_OFF, &[])?;
        }
        self.bus.delay_us(T_AF_US);
        self.powered = true;
        Ok(())
    }

    /// The power-off sequence of DS §6.9.
    pub fn power_off(&mut self) -> HalResult<()> {
        for face in Face::ALL {
            self.bus.command(face, SLEEP_ON, &[])?;
        }
        self.bus.vcc(false)?;
        self.bus.delay_us(T_OFF_US);
        self.powered = false;
        Ok(())
    }

    /// Set the RAM window to `region` (panel coordinates) and start a write.
    pub fn window(&mut self, face: Face, r: Region) -> HalResult<()> {
        if r.is_empty() || r.x1 > SIDE || r.y1 > SIDE {
            return Err(HalError::InvalidArgument);
        }
        self.bus.command(
            face,
            SET_COLUMN,
            &[COLUMN_OFFSET + r.x0, COLUMN_OFFSET + r.x1 - 1],
        )?;
        self.bus
            .command(face, SET_ROW, &[ROW_OFFSET + r.y0, ROW_OFFSET + r.y1 - 1])?;
        self.bus.command(face, WRITE_RAM, &[])
    }

    fn clear(&mut self, face: Face) -> HalResult<()> {
        self.window(face, Region::full::<Rgb64>())?;
        let zeros = [0u8; SIDE as usize * 2];
        for _ in 0..SIDE {
            self.bus.data(face, &zeros)?;
        }
        Ok(())
    }

    /// Send `region` of a packed RGB565 frame (64 × 64 × 2 bytes, rows high
    /// byte first): the window, then its rows. Horizontal increment wraps
    /// each row back to the window's first column (CT §2.1), so the rows go
    /// back to back.
    pub fn write_region(&mut self, face: Face, r: Region, frame: &[u8; 8192]) -> HalResult<()> {
        self.window(face, r)?;
        let row = SIDE as usize * 2;
        let (x0, x1) = (r.x0 as usize * 2, r.x1 as usize * 2);
        for y in r.y0 as usize..r.y1 as usize {
            self.bus.data(face, &frame[y * row + x0..y * row + x1])?;
        }
        Ok(())
    }

    /// Brightness 0–255: the master current in sixteenths (C7h) for the
    /// coarse step and the colour contrasts (C1h) scaled within it, so the
    /// white balance in [`CONTRAST_FULL`] holds. 0 turns the panel's output
    /// off (A4h, all off) without sleeping it; anything else is normal.
    pub fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()> {
        if level == 0 {
            return self.bus.command(face, DISPLAY_ALL_OFF, &[]);
        }
        // Brightness out of 256, so 255 is full.
        let l = level as u32 * 256 / 255;
        let master = l.div_ceil(16).clamp(1, 16);
        // What the master step leaves for the contrasts to make up.
        let within = (l * 16).div_ceil(master).min(256);
        let c = CONTRAST_FULL.map(|full| ((full as u32 * within) / 256).max(1) as u8);
        self.bus.command(face, MASTER_CURRENT, &[master as u8 - 1])?;
        self.bus.command(face, CONTRAST, &c)?;
        self.bus.command(face, DISPLAY_NORMAL, &[])
    }

    pub fn set_enabled(&mut self, enabled: bool) -> HalResult<()> {
        if enabled && !self.powered {
            return self.power_on();
        }
        let cmd = if enabled { SLEEP_OFF } else { SLEEP_ON };
        for face in Face::ALL {
            self.bus.command(face, cmd, &[])?;
        }
        Ok(())
    }
}

/// The board's panel bus. **Stub**: the SPIM instance, the six CS pins, D/C#,
/// RES# and the VCC enable aren't assigned yet (no 30 mm board). Every call
/// fails with `NotImplemented` through the shim, like the crate's other
/// drivers. Run SCLK at no more than [`MAX_SCLK_HZ`].
pub struct ZephyrPanels;

impl PanelBus for ZephyrPanels {
    fn command(&mut self, _face: Face, _cmd: u8, _params: &[u8]) -> HalResult<()> {
        crate::todo(|| ()) // SPIM: CS low, D/C# low, cmd; D/C# high, params; CS high
    }
    fn data(&mut self, _face: Face, _bytes: &[u8]) -> HalResult<()> {
        crate::todo(|| ()) // SPIM DMA with D/C# high
    }
    fn reset(&mut self, _low: bool) -> HalResult<()> {
        crate::todo(|| ())
    }
    fn vcc(&mut self, _on: bool) -> HalResult<()> {
        crate::todo(|| ()) // the panels' boost converter enable
    }
    fn delay_us(&mut self, _us: u32) {}
}

/// Six SSD1357 panels, as the firmware's [`Display`].
pub struct Ssd1357Array(pub Ssd1357<ZephyrPanels>);

impl<B: PanelBus> Display for Ssd1357<B> {
    type Target = Rgb64;

    fn write_frame(&mut self, face: Face, frame: &[u8; 8192]) -> HalResult<()> {
        self.write_region(face, Region::full::<Rgb64>(), frame)
    }

    fn write_region(&mut self, face: Face, region: Region, frame: &[u8; 8192]) -> HalResult<()> {
        Ssd1357::write_region(self, face, region, frame)
    }

    /// Each panel latches its RAM as it arrives; there is no swap command.
    /// Faces written in one tick land within the same few milliseconds.
    fn flush(&mut self) -> HalResult<()> {
        Ok(())
    }

    fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()> {
        Ssd1357::set_brightness(self, face, level)
    }

    fn set_enabled(&mut self, enabled: bool) -> HalResult<()> {
        Ssd1357::set_enabled(self, enabled)
    }

    /// The panels' VCC rail is the supply the magnetometer must not see.
    fn set_supply_paused(&mut self, paused: bool) -> HalResult<()> {
        self.bus.vcc(!paused)
    }
}

impl Display for Ssd1357Array {
    type Target = Rgb64;

    fn write_frame(&mut self, face: Face, frame: &[u8; 8192]) -> HalResult<()> {
        self.0.write_frame(face, frame)
    }
    fn write_region(&mut self, face: Face, region: Region, frame: &[u8; 8192]) -> HalResult<()> {
        Display::write_region(&mut self.0, face, region, frame)
    }
    fn flush(&mut self) -> HalResult<()> {
        self.0.flush()
    }
    fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()> {
        Display::set_brightness(&mut self.0, face, level)
    }
    fn set_enabled(&mut self, enabled: bool) -> HalResult<()> {
        Display::set_enabled(&mut self.0, enabled)
    }
    fn set_supply_paused(&mut self, paused: bool) -> HalResult<()> {
        self.0.set_supply_paused(paused)
    }
}

const _: () = assert!(FACE_COUNT == 6);

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    #[derive(Debug, PartialEq, Clone)]
    enum Op {
        Cmd(usize, u8, Vec<u8>),
        Data(usize, Vec<u8>),
        Reset(bool),
        Vcc(bool),
        Delay(u32),
    }

    #[derive(Default)]
    struct Log(Vec<Op>);

    impl PanelBus for Log {
        fn command(&mut self, face: Face, cmd: u8, params: &[u8]) -> HalResult<()> {
            self.0.push(Op::Cmd(face.index(), cmd, params.to_vec()));
            Ok(())
        }
        fn data(&mut self, face: Face, bytes: &[u8]) -> HalResult<()> {
            self.0.push(Op::Data(face.index(), bytes.to_vec()));
            Ok(())
        }
        fn reset(&mut self, low: bool) -> HalResult<()> {
            self.0.push(Op::Reset(low));
            Ok(())
        }
        fn vcc(&mut self, on: bool) -> HalResult<()> {
            self.0.push(Op::Vcc(on));
            Ok(())
        }
        fn delay_us(&mut self, us: u32) {
            self.0.push(Op::Delay(us));
        }
    }

    #[test]
    fn power_on_follows_the_datasheet_sequence() {
        let mut p = Ssd1357::new(Log::default());
        p.power_on().unwrap();
        let ops = &p.bus.0;
        // VDD settles ≥ 20 ms, RES# low ≥ 3 µs, VCC on, RES# high, ≥ 300 ms
        // from VDD before the first command.
        assert_eq!(ops[0], Op::Delay(20_000));
        assert_eq!(ops[1], Op::Reset(true));
        assert_eq!(ops[2], Op::Delay(3));
        assert_eq!(ops[3], Op::Vcc(true));
        assert_eq!(ops[4], Op::Reset(false));
        assert_eq!(ops[5], Op::Delay(280_000));
        // Unlock first, display last, then tAF.
        assert_eq!(ops[6], Op::Cmd(0, COMMAND_LOCK, std::vec![0x12]));
        assert_eq!(*ops.last().unwrap(), Op::Delay(200_000));
        let wakes: Vec<_> = ops.iter().filter(|o| matches!(o, Op::Cmd(_, 0xAF, _))).collect();
        assert_eq!(wakes.len(), 6);
        // 65k colour, horizontal increment.
        assert!(ops.contains(&Op::Cmd(0, REMAP, std::vec![0b0110_0000, 0])));
        assert!(ops.contains(&Op::Cmd(5, MUX_RATIO, std::vec![63])));
    }

    #[test]
    fn a_region_sets_the_window_then_sends_its_rows() {
        let mut p = Ssd1357::new(Log::default());
        let mut frame = [0u8; 8192];
        // Pixel (9, 2) red, high byte first.
        frame[(2 * 64 + 9) * 2] = 0xF8;
        let r = Region {
            x0: 8,
            y0: 0,
            x1: 16,
            y1: 8,
        };
        Ssd1357::write_region(&mut p, Face::PosZ, r, &frame).unwrap();
        let ops = &p.bus.0;
        assert_eq!(ops[0], Op::Cmd(4, SET_COLUMN, std::vec![8, 15]));
        assert_eq!(ops[1], Op::Cmd(4, SET_ROW, std::vec![0, 7]));
        assert_eq!(ops[2], Op::Cmd(4, WRITE_RAM, std::vec![]));
        let rows: Vec<&Vec<u8>> = ops[3..]
            .iter()
            .map(|o| match o {
                Op::Data(4, d) => d,
                _ => panic!("{o:?}"),
            })
            .collect();
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|r| r.len() == 16));
        assert_eq!(&rows[2][2..4], &[0xF8, 0x00]);
    }

    #[test]
    fn a_region_outside_the_panel_is_refused() {
        let mut p = Ssd1357::new(Log::default());
        let r = Region {
            x0: 0,
            y0: 0,
            x1: 65,
            y1: 1,
        };
        assert_eq!(p.window(Face::PosX, r), Err(HalError::InvalidArgument));
    }

    #[test]
    fn brightness_scales_master_and_contrast() {
        let mut p = Ssd1357::new(Log::default());
        p.set_brightness(Face::PosX, 255).unwrap();
        assert_eq!(p.bus.0[0], Op::Cmd(0, MASTER_CURRENT, std::vec![15]));
        assert_eq!(p.bus.0[1], Op::Cmd(0, CONTRAST, CONTRAST_FULL.to_vec()));
        p.bus.0.clear();
        p.set_brightness(Face::PosX, 128).unwrap();
        // Half: 8/16 master, contrasts at full.
        assert_eq!(p.bus.0[0], Op::Cmd(0, MASTER_CURRENT, std::vec![7]));
        assert_eq!(p.bus.0[1], Op::Cmd(0, CONTRAST, CONTRAST_FULL.to_vec()));
        p.bus.0.clear();
        p.set_brightness(Face::PosX, 0).unwrap();
        assert_eq!(p.bus.0, std::vec![Op::Cmd(0, DISPLAY_ALL_OFF, std::vec![])]);
    }

    #[test]
    fn sleep_and_wake() {
        let mut p = Ssd1357::new(Log::default());
        p.power_on().unwrap();
        p.bus.0.clear();
        p.set_enabled(false).unwrap();
        assert!(p.bus.0.iter().all(|o| matches!(o, Op::Cmd(_, SLEEP_ON, _))));
        p.bus.0.clear();
        p.set_enabled(true).unwrap();
        assert!(p.bus.0.iter().all(|o| matches!(o, Op::Cmd(_, SLEEP_OFF, _))));
        p.power_off().unwrap();
        assert!(p.bus.0.contains(&Op::Vcc(false)));
    }
}
