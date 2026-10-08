//! One running cartridge and the cube around it, a frame at a time.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::color::palette_from_ram;
use gbc_cube_core::crystal;
use gbc_cube_core::cube::{Config, Cube, FrameIn, Report};
use gbc_cube_core::view::V;
use gbc_cube_core::{FaceBuf, LCD_H, LCD_W};
use gbc_cube_demo::Demo;
use gbc_cube_emu::{Emulator, RomStats};
use smokebomb_hal::ImuSample;

/// What to run.
#[derive(Clone, Debug)]
pub enum Cart {
    /// A ROM file; its battery save goes next to it.
    Rom(PathBuf),
    /// The built-in stand-in for Crystal (no ROM needed).
    Demo,
}

impl Cart {
    /// `--rom PATH`, else `GBC_CUBE_ROM`, else `--demo` / the demo.
    pub fn from_args(rom: Option<PathBuf>, demo: bool) -> Cart {
        if demo {
            return Cart::Demo;
        }
        match rom.or_else(|| std::env::var_os("GBC_CUBE_ROM").map(PathBuf::from)) {
            Some(p) => Cart::Rom(p),
            None => Cart::Demo,
        }
    }
}

/// The sensors and buttons for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub imu: ImuSample,
    pub touch: u8,
    /// Buttons pressed directly (keyboard), on top of the cube's.
    pub keys: Buttons,
}

pub struct FrameOut<'a> {
    pub faces: &'a [FaceBuf; 6],
    pub frame: &'a [u16; LCD_W * LCD_H],
    /// The world renderer's picture of this frame's screen (drawn from the
    /// previous frame's RAM), when it drew the map.
    pub predicted: Option<&'a [u16]>,
    pub mismatches: Option<usize>,
    pub report: Report,
    pub buttons: Buttons,
    pub emu_us: u32,
    pub render_us: u32,
}

pub struct Session {
    pub emu: Emulator,
    pub demo: Option<Demo>,
    pub cube: Box<Cube>,
    pub title: String,
    pub crystal: bool,
    faces: Box<[FaceBuf; 6]>,
    predicted: Vec<u16>,
    have_prediction: bool,
    next_prediction: Vec<u16>,
    started: Instant,
    pub frames: u64,
}

impl Session {
    pub fn open(cart: &Cart, cfg: Config) -> Result<Session> {
        let (mut emu, demo) = match cart {
            Cart::Rom(path) => {
                let save = Emulator::default_save_path(path);
                let emu = Emulator::open(path, Some(save))
                    .with_context(|| format!("loading {}", path.display()))?;
                (emu, None)
            }
            Cart::Demo => (Emulator::new(gbc_cube_demo::rom())?, Some(Demo::new())),
        };
        if !emu.cpu().cgb_mode {
            // Fine for benchmarking the CPU; the cube's colours assume CGB.
            tracing::warn!(
                "{} isn't a Game Boy Color game: running it in DMG mode",
                emu.title()
            );
        }
        let title = emu.title();
        let crystal = demo.is_some() || crystal::is_crystal(&title);
        let mut demo = demo;
        if let Some(d) = &mut demo {
            d.boot(&mut emu);
        }
        Ok(Session {
            emu,
            demo,
            cube: Box::new(Cube::new(cfg)),
            title,
            crystal,
            faces: Box::new([[0; 4096]; 6]),
            predicted: vec![0; LCD_W * LCD_H],
            have_prediction: false,
            next_prediction: vec![0; LCD_W * LCD_H],
            started: Instant::now(),
            frames: 0,
        })
    }

    pub fn now_ms(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    /// Run one frame: read the sensors, run the emulator, draw the faces.
    pub fn tick(&mut self, input: &Input) -> Result<FrameOut<'_>> {
        let now = self.now_ms();
        let buttons = self.cube.sense(now, &input.imu, input.touch) | input.keys;
        self.emu.set_buttons(buttons);
        let t0 = Instant::now();
        self.emu.run_frame()?;
        let emu_us = t0.elapsed().as_micros() as u32;
        self.frames += 1;

        // How the last prediction did against the frame it predicted.
        let mismatches = self.have_prediction.then(|| {
            let f = self.emu.frame();
            (0..LCD_W * LCD_H)
                .filter(|&i| f[i] != self.next_prediction[i])
                .count()
        });
        if self.have_prediction {
            std::mem::swap(&mut self.predicted, &mut self.next_prediction);
        }
        let had_prediction = self.have_prediction;

        let t1 = Instant::now();
        let mem = self.emu.mem();
        let palette = palette_from_ram(
            gbc_cube_core::mem::GbMem::bg_palette(&mem),
            gbc_cube_core::mem::GbMem::obj_palette(&mem),
        );
        let input_frame = FrameIn {
            mem: &mem,
            frame: self.emu.frame_index(),
            palette: &palette,
            crystal: self.crystal,
        };
        let report = self.cube.render(&input_frame, &mut self.faces);
        let render_us = t1.elapsed().as_micros() as u32;

        // The world renderer's view of the screen region, for the next frame.
        self.have_prediction = false;
        if let Some((ox, oy)) = report.screen_on_canvas {
            let view = &self.cube.view;
            for y in 0..LCD_H {
                for x in 0..LCD_W {
                    let (cx, cy) = (ox + x as i32, oy + y as i32);
                    self.next_prediction[y * LCD_W + x] =
                        if (0..V as i32).contains(&cx) && (0..V as i32).contains(&cy) {
                            view.raw(cx as usize, cy as usize)
                        } else {
                            0
                        };
                }
            }
            self.have_prediction = true;
        }

        if let Some(d) = &mut self.demo {
            d.step(&mut self.emu, buttons);
        }
        let _ = self.emu.flush_save(false);
        Ok(FrameOut {
            faces: &self.faces,
            frame: self.emu.frame(),
            predicted: had_prediction.then_some(&self.predicted[..]),
            mismatches,
            report,
            buttons,
            emu_us,
            render_us,
        })
    }

    pub fn take_rom_stats(&mut self) -> RomStats {
        self.emu.take_stats()
    }
}
