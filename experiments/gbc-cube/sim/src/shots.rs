//! Headless screenshots: the six faces unfolded as a net next to the
//! emulator's own frame, for a scripted run of the demo cart (or a ROM).
//!
//! The net is drawn the way the map drapes: the up face in the middle and
//! each side face folded out flat beyond its edge (so its top touches the
//! up face), with a strip of table between faces. The bottom face (black,
//! on the table) is left out. Below it, the five faces as a person reads
//! them, each upright: up, front (south), east, back (north), west.

use std::path::Path;

use anyhow::Result;
use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::cube::{Config, Wrap};
use gbc_cube_core::fallback::UiStyle;
use gbc_cube_core::geom::{drape, Compass, Layout, Role};
use gbc_cube_core::{FaceBuf, FACE, LCD_H, LCD_W};

use crate::image::{rgb, Canvas};
use crate::session::{Cart, Input, Session};

const SCALE: i32 = 3;
const GAP: i32 = 2;

/// The net (192×192 plus gaps) and the 160×144 frame, side by side.
pub fn compose(faces: &[FaceBuf; 6], layout: &Layout, frame: &[u16]) -> Canvas {
    let net = (FACE as i32 * 3 + 2 * GAP) * SCALE;
    let pad = 12;
    let strip_scale = 2;
    let strip = FACE as i32 * strip_scale;
    let w = (pad * 3 + net + LCD_W as i32 * SCALE) as usize;
    let h = (pad * 3 + net + strip) as usize;
    let mut c = Canvas::new(w, h, [28, 28, 32]);
    let mid = FACE as i32 * 3 / 2 + GAP;
    for (i, face) in faces.iter().enumerate() {
        let role = layout.role[i];
        let shift = match role {
            Role::Side(d) => (d.step().0 * GAP, d.step().1 * GAP),
            _ => (0, 0),
        };
        if role == Role::Bottom {
            continue;
        }
        let xf = layout.xf[i];
        for v in 0..FACE {
            for u in 0..FACE {
                let (dx, dy) = drape(role, u as i32, v as i32).expect("not the bottom");
                let p = face[xf.index(u, v)];
                c.put((pad, pad), SCALE, mid + dx + shift.0, mid + dy + shift.1, rgb(p));
            }
        }
    }
    // Each face upright, as seen.
    let order = [
        Role::Top,
        Role::Side(Compass::South),
        Role::Side(Compass::East),
        Role::Side(Compass::North),
        Role::Side(Compass::West),
    ];
    for (k, role) in order.into_iter().enumerate() {
        let i = layout.face_with(role).index();
        let origin = (pad + k as i32 * (strip + pad), pad * 2 + net);
        for v in 0..FACE {
            for u in 0..FACE {
                let p = faces[i][layout.xf[i].index(u, v)];
                c.put(origin, strip_scale, u as i32, v as i32, rgb(p));
            }
        }
    }
    let fx = pad * 2 + net;
    let fy = pad + (net - LCD_H as i32 * SCALE) / 2;
    for y in 0..LCD_H {
        for x in 0..LCD_W {
            c.put((fx, fy), SCALE, x as i32, y as i32, rgb(frame[y * LCD_W + x]));
        }
    }
    c
}

struct Shooter<'a> {
    s: Session,
    out: &'a Path,
    input: Input,
}

impl Shooter<'_> {
    fn run(&mut self, frames: usize, keys: Buttons) -> Result<()> {
        self.input.keys = keys;
        for _ in 0..frames {
            self.s.tick(&self.input)?;
        }
        self.input.keys = Buttons::NONE;
        Ok(())
    }

    /// Tap a button for a few frames, then let go for a few.
    fn tap(&mut self, b: Buttons) -> Result<()> {
        self.run(4, b)?;
        self.run(4, Buttons::NONE)
    }

    /// Run a frame with `cfg` (a few, so A's pan settles) and save.
    fn shot(&mut self, name: &str, wrap: Wrap, ui: UiStyle, settle: usize) -> Result<()> {
        let keep = self.s.cube.cfg;
        self.s.cube.cfg.wrap = wrap;
        self.s.cube.cfg.ui = ui;
        let mut img = None;
        for _ in 0..settle.max(1) {
            let out = self.s.tick(&self.input)?;
            let layout = Layout::new(out.report.heading);
            img = Some(compose(out.faces, &layout, out.frame));
        }
        let path = self.out.join(format!("{name}.png"));
        img.expect("at least one frame").save(&path)?;
        println!("  {}", path.display());
        self.s.cube.cfg = keep;
        Ok(())
    }
}

pub fn run(cart: &Cart, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let s = Session::open(cart, Config::default())?;
    let level = smokebomb_hal::ImuSample {
        accel_mg: [0, 1000, 0],
        gyro_mdps: [0; 3],
    };
    let mut sh = Shooter {
        s,
        out,
        input: Input {
            imu: level,
            ..Input::default()
        },
    };
    println!(
        "{} ({})",
        sh.s.title,
        if sh.s.crystal {
            "Crystal layout"
        } else {
            "frame only"
        }
    );
    if !matches!(cart, Cart::Demo) {
        // A ROM: just the title screen and whatever follows, both ways.
        sh.run(900, Buttons::NONE)?;
        sh.shot("rom-frame", Wrap::Frame, UiStyle::Front, 1)?;
        sh.shot("rom-world", Wrap::World, UiStyle::Front, 1)?;
        return Ok(());
    }
    use Buttons as B;
    // Overworld: up the road between the pond and the house, then stand.
    sh.run(20, B::NONE)?;
    sh.run(16 * 6 + 4, B::UP)?;
    sh.run(20, B::NONE)?;
    sh.shot("01-overworld-world", Wrap::World, UiStyle::Front, 1)?;
    sh.shot("02-overworld-frame", Wrap::Frame, UiStyle::Front, 1)?;
    // Mid-step, scrolling right.
    sh.run(9, B::RIGHT)?;
    sh.shot("03-mid-step-world", Wrap::World, UiStyle::Front, 1)?;
    sh.run(30, B::NONE)?;
    sh.run(16, B::LEFT)?;
    sh.run(20, B::NONE)?;
    // On to the north edge of town: the next route's strip shows past it,
    // fog where there's no map.
    sh.run(16 * 12 + 4, B::UP)?;
    sh.run(20, B::NONE)?;
    sh.shot("04-north-edge-world", Wrap::World, UiStyle::Front, 1)?;
    sh.shot("05-north-edge-frame", Wrap::Frame, UiStyle::Front, 1)?;
    // Down the road and west to the edge of town, which connects to
    // nothing: fog past the trees.
    sh.run(16 * 18 + 4, B::DOWN)?;
    sh.run(16 * 16 + 4, B::LEFT)?;
    sh.run(20, B::NONE)?;
    sh.shot("06-west-edge-world", Wrap::World, UiStyle::Front, 1)?;
    // Start menu.
    sh.tap(B::START)?;
    sh.tap(B::DOWN)?;
    sh.run(10, B::NONE)?;
    for (name, ui) in [
        ("07-menu-c-front", UiStyle::Front),
        ("08-menu-a-pan", UiStyle::Pan),
        ("09-menu-b-spread", UiStyle::Spread),
    ] {
        sh.shot(name, Wrap::World, ui, 40)?;
    }
    // A text box, half typed and finished.
    sh.tap(B::A)?;
    sh.run(14, B::NONE)?;
    sh.shot("10-text-typing-c-front", Wrap::World, UiStyle::Front, 1)?;
    sh.shot("11-text-typing-a-pan", Wrap::World, UiStyle::Pan, 1)?;
    sh.run(60, B::NONE)?;
    for (name, ui) in [
        ("12-text-c-front", UiStyle::Front),
        ("13-text-a-pan", UiStyle::Pan),
        ("14-text-b-spread", UiStyle::Spread),
    ] {
        sh.shot(name, Wrap::World, ui, 40)?;
    }
    sh.tap(B::A)?;
    sh.run(10, B::NONE)?;
    // A battle (Select starts one in the demo).
    sh.tap(B::SELECT)?;
    sh.run(80, B::NONE)?;
    sh.tap(B::A)?;
    sh.run(10, B::NONE)?;
    for (name, ui) in [
        ("15-battle-c-front", UiStyle::Front),
        ("16-battle-a-pan", UiStyle::Pan),
        ("17-battle-b-spread", UiStyle::Spread),
    ] {
        sh.shot(name, Wrap::World, ui, 40)?;
    }
    Ok(())
}
