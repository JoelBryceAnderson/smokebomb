//! Headless benchmark: how long the emulator and the cube renderer take per
//! frame on this machine, and what the ROM side does (bank switches, banks
//! touched), for `GBC_CUBE_HW_FEASIBILITY.md`.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::crystal::world::{self, BlockCache, Camera, MapInfo};
use gbc_cube_core::cube::Config;
use gbc_cube_core::geom::{Heading, Layout};
use gbc_cube_core::view::View;
use gbc_cube_core::FaceBuf;
use serde_json::json;

use crate::session::{Cart, Input, Session};

fn stats(mut v: Vec<u32>) -> serde_json::Value {
    if v.is_empty() {
        return json!(null);
    }
    v.sort_unstable();
    let n = v.len();
    let mean = v.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    json!({
        "mean_us": (mean * 10.0).round() / 10.0,
        "p50_us": v[n / 2],
        "p99_us": v[(n * 99 / 100).min(n - 1)],
        "max_us": v[n - 1],
        "samples": n,
    })
}

/// Joypad for frame `i`: the demo strolls a square; a ROM gets A pressed
/// through the intro and then wanders.
fn script(i: u64, demo: bool) -> Buttons {
    if demo {
        return match (i / 96) % 5 {
            0 => Buttons::RIGHT,
            1 => Buttons::DOWN,
            2 => Buttons::LEFT,
            3 => Buttons::UP,
            _ => Buttons::NONE,
        };
    }
    if i < 1200 {
        return if i % 60 < 4 { Buttons::A } else { Buttons::NONE };
    }
    match (i / 64) % 6 {
        0 => Buttons::UP,
        1 => Buttons::RIGHT,
        2 => Buttons::DOWN,
        3 => Buttons::LEFT,
        4 => {
            if i % 32 < 4 {
                Buttons::B
            } else {
                Buttons::NONE
            }
        }
        _ => Buttons::NONE,
    }
}

pub fn run(cart: &Cart, frames: u64, json_out: Option<&Path>) -> Result<()> {
    let mut s = Session::open(cart, Config::default())?;
    let demo = s.demo.is_some();
    let level = smokebomb_hal::ImuSample {
        accel_mg: [0, 1000, 0],
        gyro_mdps: [0; 3],
    };
    let mut input = Input {
        imu: level,
        ..Input::default()
    };
    let (mut emu, mut cube, mut draw, mut drape) = (vec![], vec![], vec![], vec![]);
    let (mut double_speed, mut world_frames) = (0u64, 0u64);
    let mut view = Box::new(View::new());
    let mut cache = BlockCache::new();
    let mut faces = Box::new([[0u16; 4096]; 6] as [FaceBuf; 6]);
    let layout = Layout::new(Heading::default());
    s.take_rom_stats();
    let wall = Instant::now();
    for i in 0..frames {
        input.keys = script(i, demo);
        let out = s.tick(&input)?;
        emu.push(out.emu_us);
        cube.push(out.render_us);
        if s.emu.cpu().double_speed {
            double_speed += 1;
        }
        // The two halves of the world path on their own: drawing the map
        // canvas, and draping it over five faces.
        if s.crystal {
            let mem = s.emu.mem();
            if let Some(map) = MapInfo::read(&mem) {
                if let Some(cam) = Camera::read(&mem, &map) {
                    let t = Instant::now();
                    world::draw(&mem, &map, &cam, &mut cache, &mut view);
                    view.compute_light(16);
                    draw.push(t.elapsed().as_micros() as u32);
                    world_frames += 1;
                    let t = Instant::now();
                    view.drape_onto(
                        &layout,
                        &mut faces,
                        1 << layout.face_with(gbc_cube_core::geom::Role::Bottom).index(),
                    );
                    drape.push(t.elapsed().as_micros() as u32);
                }
            }
        }
    }
    let secs = wall.elapsed().as_secs_f64();
    let rom = s.take_rom_stats();
    let f = rom.frames.max(1) as f64;
    // Which banks would have to sit in fast memory: the fewest banks that
    // cover 50 / 90 / 99 / 100 % of all ROM reads (bank 0 included).
    let mut reads: Vec<(usize, u32)> = rom
        .bank_reads
        .iter()
        .copied()
        .enumerate()
        .filter(|b| b.1 > 0)
        .collect();
    reads.sort_by_key(|b| std::cmp::Reverse(b.1));
    let total: u64 = reads.iter().map(|b| b.1 as u64).sum();
    let cover = |frac: f64| {
        let mut acc = 0u64;
        reads
            .iter()
            .take_while(|b| {
                let before = acc;
                acc += b.1 as u64;
                (before as f64) < frac * total as f64
            })
            .count()
    };
    let hottest: Vec<String> = reads
        .iter()
        .take(12)
        .map(|(b, n)| format!("${b:02X}:{:.1}%", *n as f64 * 100.0 / total.max(1) as f64))
        .collect();
    let report = json!({
        "title": s.title,
        "frames": frames,
        "wall_s": (secs * 100.0).round() / 100.0,
        "speed_x_realtime": ((frames as f64 / gbc_cube_emu::FRAME_HZ) / secs * 10.0).round() / 10.0,
        "emulator": stats(emu),
        "cube_render": stats(cube),
        "world_draw": stats(draw),
        "drape_5_faces": stats(drape),
        "world_frames": world_frames,
        "double_speed_frames": double_speed,
        "rom": {
            "reads_per_frame": (rom.rom_reads as f64 / f).round(),
            "bank_changes_per_frame": (rom.bank_changes as f64 / f * 10.0).round() / 10.0,
            "banks_touched": rom.banks_touched,
            "cart_ram_writes": rom.cart_ram_writes,
            "banks_for_50pct_of_reads": cover(0.5),
            "banks_for_90pct_of_reads": cover(0.9),
            "banks_for_99pct_of_reads": cover(0.99),
            "hottest_banks": hottest,
        },
        "host": std::env::consts::ARCH,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(p) = json_out {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(())
}
