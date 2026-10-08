//! The Phase 3 world renderer against the emulator's real PPU.
//!
//! The demo cart writes Crystal-layout state; the renderer draws the map from
//! it one frame ahead (see `crystal::world`). Every pixel of the 160×144
//! screen region of that drawing must equal the PPU's next frame: terrain,
//! both VRAM banks, the palette map, the smooth step scroll, objects with
//! their walk frames and flips, and object/BG priority.
//!
//! One known difference: tile animations (water, flowers) are written to
//! VRAM by the VBlank handler, after the renderer read VRAM, so on the frame
//! an animation steps the renderer still has the old graphics. Those frames
//! are counted, not failed.

use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::crystal::screen::{self, Scene};
use gbc_cube_core::crystal::world::{self, BlockCache, Camera, MapInfo};
use gbc_cube_core::view::{View, V, VOID};
use gbc_cube_core::{LCD_H, LCD_W};
use gbc_cube_demo::{rom, Demo};
use gbc_cube_emu::Emulator;

#[test]
fn every_screen_pixel_matches_the_next_ppu_frame() {
    let mut emu = Emulator::new(rom()).unwrap();
    let mut demo = Demo::new();
    demo.boot(&mut emu);
    let mut cache = BlockCache::new();
    let mut view = Box::new(View::new());
    // A walk: right, down, left, up, through grass, past the house, with
    // the townsfolk wandering.
    let mut script = vec![];
    for (b, frames) in [
        (Buttons::NONE, 30),
        (Buttons::RIGHT, 80),
        (Buttons::DOWN, 70),
        (Buttons::LEFT, 120),
        (Buttons::UP, 150),
        (Buttons::RIGHT, 40),
        (Buttons::NONE, 30),
    ] {
        script.extend(std::iter::repeat_n(b, frames));
    }
    let mut expected: Option<Vec<u16>> = None;
    let (mut checked, mut moving_frames, mut animation_frames) = (0, 0, 0);
    let start = demo.player();
    for (i, &b) in script.iter().enumerate() {
        emu.run_frame().unwrap();
        if let Some(want) = expected.take() {
            let got = emu.frame();
            let bad: Vec<usize> = (0..LCD_W * LCD_H).filter(|&p| got[p] != want[p]).collect();
            if !bad.is_empty() && demo.animated() {
                animation_frames += 1;
            } else {
                assert!(
                    bad.is_empty(),
                    "frame {i}: {} pixels differ, first at ({}, {}): ppu {:04X} renderer {:04X}",
                    bad.len(),
                    bad[0] % LCD_W,
                    bad[0] / LCD_W,
                    got[bad[0]],
                    want[bad[0]],
                );
            }
            checked += 1;
        }
        let mem = emu.mem();
        let map = MapInfo::read(&mem).expect("map");
        let cam = Camera::read(&mem, &map).expect("camera");
        let info = screen::classify(&mem, Some(&map), Some(&cam), &mut cache);
        assert_eq!(info.scene, Scene::Overworld, "frame {i}: {info:?}");
        if (cam.screen.0 % 16, cam.screen.1 % 16) != (0, 0) {
            moving_frames += 1;
        }
        assert!(gbc_cube_core::crystal::settled(&mem), "waiting for VBlank");
        world::draw(&mem, &map, &cam, &mut cache, &mut view);
        let (ox, oy) = cam.screen_on_canvas();
        let mut want = vec![0u16; LCD_W * LCD_H];
        for y in 0..LCD_H {
            for x in 0..LCD_W {
                let (cx, cy) = (ox + x as i32, oy + y as i32);
                assert!((0..V as i32).contains(&cx) && (0..V as i32).contains(&cy));
                let p = view.idx[cy as usize * V + cx as usize];
                assert_ne!(p, VOID, "frame {i}: screen pixel ({x}, {y}) is fog");
                want[y * LCD_W + x] = view.raw(cx as usize, cy as usize);
            }
        }
        expected = Some(want);
        demo.step(&mut emu, b);
    }
    assert!(checked > 400);
    assert!(
        animation_frames <= script.len() / 32 + 1,
        "{animation_frames} frames off"
    );
    assert!(
        moving_frames > 100,
        "the walk scrolled ({moving_frames} mid-step frames)"
    );
    assert_ne!(demo.player(), start, "the player moved");
}
