//! The GBC-on-a-cube experiment's hardware-agnostic logic.
//!
//! `no_std` and allocation-free, like the firmware core, so it could run on
//! the die's nRF54L15 next to the emulator. It never touches the emulator
//! directly: it reads console memory through [`mem::GbMem`] and the last
//! frame as palette indexes, and writes six 64×64 RGB565 face buffers.
//!
//! * [`geom`]: which face is up, where north is, how the map drapes over the
//!   faces.
//! * [`view`]: the 192×192 canvas the faces sample (the up face's 64×64 plus
//!   64 px beyond each edge), with fog where there's nothing to show.
//! * [`crystal`]: Pokémon Crystal's overworld redrawn from game RAM (Phase 3),
//!   and what the screen is showing (overworld, a text box, a battle).
//! * [`fallback`]: layouts for screens that don't wrap (Phase 4).
//! * [`screens`]: Crystal's naming screen and still scenes (the new-game
//!   speech, the title), laid out for the cube.
//! * [`controls`]: tilt, taps, long press and shake to joypad (Phase 5).
//! * [`cube`]: puts it together, once per frame.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod buttons;
pub mod color;
pub mod controls;
pub mod crystal;
pub mod cube;
pub mod fallback;
pub mod geom;
pub mod mem;
pub mod orient;
pub mod ppu;
pub mod screens;
pub mod view;

/// The Game Boy screen.
pub const LCD_W: usize = 160;
pub const LCD_H: usize = 144;
/// A face of the 30 mm die.
pub const FACE: usize = 64;
pub const FACE_PIXELS: usize = FACE * FACE;

/// One face's pixels, RGB565, in the face's drawing axes (row-major, row 0
/// at the top of the face's canvas).
pub type FaceBuf = [u16; FACE_PIXELS];
