//! `gbc-cube snapshot --out DIR`: the demo cart's memory at two moments
//! (mid-step in the overworld, and with a text box open), for the
//! Cortex-M33 benchmark (`m33bench/`) to render without an emulator host.

use std::path::Path;

use anyhow::Result;
use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::cube::Config;
use gbc_cube_core::mem::GbMem;

use crate::session::{Cart, Input, Session};

fn dump(s: &Session, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let m = s.emu.mem();
    std::fs::write(dir.join("wram.bin"), m.wram())?;
    std::fs::write(dir.join("vram.bin"), m.vram())?;
    std::fs::write(dir.join("oam.bin"), m.oam())?;
    std::fs::write(dir.join("io.bin"), m.io())?;
    std::fs::write(dir.join("bgpal.bin"), m.bg_palette())?;
    std::fs::write(dir.join("objpal.bin"), m.obj_palette())?;
    std::fs::write(dir.join("frame.bin"), s.emu.frame_index())?;
    Ok(())
}

pub fn run(out: &Path) -> Result<()> {
    let mut s = Session::open(&Cart::Demo, Config::default())?;
    let level = smokebomb_hal::ImuSample {
        accel_mg: [0, 1000, 0],
        gyro_mdps: [0; 3],
    };
    let mut input = Input {
        imu: level,
        ..Input::default()
    };
    let mut run = |s: &mut Session, frames: usize, keys: Buttons| -> Result<()> {
        input.keys = keys;
        for _ in 0..frames {
            s.tick(&input)?;
        }
        Ok(())
    };
    // Up the road past the pond, stopping mid-step.
    run(&mut s, 20, Buttons::NONE)?;
    run(&mut s, 16 * 5 + 9, Buttons::UP)?;
    dump(&s, &out.join("walk"))?;
    run(&mut s, 30, Buttons::NONE)?;
    // Start, then a menu item: a text box.
    run(&mut s, 4, Buttons::START)?;
    run(&mut s, 6, Buttons::NONE)?;
    run(&mut s, 4, Buttons::A)?;
    run(&mut s, 80, Buttons::NONE)?;
    dump(&s, &out.join("text"))?;
    std::fs::write(out.join("rom.bin"), s.emu.rom())?;
    println!("wrote {}", out.display());
    Ok(())
}
