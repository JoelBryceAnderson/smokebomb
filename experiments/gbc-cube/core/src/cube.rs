//! Once per frame: sensors in, joypad out, six faces drawn.

use smokebomb_core::orientation::Gravity;
use smokebomb_hal::ImuSample;

use crate::buttons::Buttons;
use crate::controls::{ControlConfig, Controls, Sense};
use crate::crystal::screen::{self, Scene, ScreenInfo};
use crate::crystal::world::{self, BlockCache, Camera, MapInfo};
use crate::fallback::{self, Fallback, UiStyle};
use crate::geom::{Heading, Layout, Role};
use crate::mem::GbMem;
use crate::orient::{UpConfig, UpTracker};
use crate::screens;
use crate::view::View;
use crate::{FaceBuf, LCD_H, LCD_W};

/// How the overworld goes round the cube.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    /// Phase 2: the emulator's 160×144 frame, cropped and folded. Past the
    /// frame's edges is fog.
    Frame,
    /// Phase 3: the map redrawn from RAM (Crystal only), real map on every
    /// side face.
    #[default]
    World,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub wrap: Wrap,
    pub ui: UiStyle,
    /// Fog depth, Game Boy pixels.
    pub fog_px: u8,
    /// Frame pixel at the up face's centre when the game can't say where the
    /// player is (any game but Crystal).
    pub default_centre: (i32, i32),
    pub up: UpConfig,
    pub controls: ControlConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wrap: Wrap::World,
            ui: UiStyle::Front,
            fog_px: 16,
            // Crystal's player cell (see `Camera::read`); a fair guess for
            // other top-down games.
            default_centre: (72, 72),
            up: UpConfig::default(),
            controls: ControlConfig::default(),
        }
    }
}

/// One finished frame from the emulator.
pub struct FrameIn<'a, M: GbMem + ?Sized> {
    pub mem: &'a M,
    /// The PPU's output as palette indexes (0–31 BG, 32–63 OBJ).
    pub frame: &'a [u8; LCD_W * LCD_H],
    /// The colours those indexes had when the frame was drawn.
    pub palette: &'a [u16; 64],
    /// The ROM is Pokémon Crystal (or lays its RAM out the same way): RAM
    /// can be read.
    pub crystal: bool,
}

/// What [`Cube::render`] drew, for the debug view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawn {
    /// Phase 3 world (the canvas is the map).
    World,
    /// Phase 2: the frame folded round the player.
    Frame,
    /// Phase 4 A.
    Pan,
    /// Phase 4 B.
    Spread,
    /// Phase 4 C: world (or battle crops) plus the front panel.
    Front,
    /// C: the naming screen's keyboard and name.
    Naming,
    /// C: a still screen folded round the cube, its box on the front.
    Still,
}

#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub scene: Scene,
    pub drawn: Drawn,
    pub camera: Option<Camera>,
    /// Canvas position of the screen's top-left when the canvas holds the
    /// map (for comparing the world renderer with the next frame).
    pub screen_on_canvas: Option<(i32, i32)>,
    pub info: ScreenInfo,
    pub heading: Heading,
    /// The canvas was kept from an earlier frame (the game was mid-update).
    pub reused: bool,
}

impl Report {
    /// Whether the next frame's pixels from the emulator will be wanted.
    /// The world and C's text panel are drawn from RAM and VRAM, so in the
    /// overworld they aren't, and the emulator can skip drawing lines (on
    /// the M33 that's about 1 M of its instructions a frame). Phase 2, A, B
    /// and the battle crops copy the frame. When the screen switches to one
    /// of those, its first frame is a frame old.
    pub fn needs_frame(&self) -> bool {
        match self.drawn {
            Drawn::World | Drawn::Naming => false,
            Drawn::Front => self.scene == Scene::Battle,
            Drawn::Frame | Drawn::Pan | Drawn::Spread | Drawn::Still => true,
        }
    }
}

pub struct Cube {
    pub cfg: Config,
    gravity: Gravity,
    tracker: UpTracker,
    controls: Controls,
    /// The canvas the faces sample.
    pub view: View,
    cache: BlockCache,
    fallback: Fallback,
    last_world: Option<Camera>,
    last_ms: Option<u32>,
    /// The scene last drawn, and how many frames in a row RAM has said
    /// it's no longer the map (see [`HOLD_FRAMES`]).
    scene: Scene,
    other_run: u8,
}

/// Frames the map must be gone before the cube stops drawing it. Walking
/// across a map connection or a step can leave Crystal's RAM between two
/// states for a frame (new blocks, old anchor), which reads as "not the
/// map"; without this the faces flash to the fallback for that frame.
/// Genuinely leaving the map (the bag, a warp's fade) waits 100 ms.
pub const HOLD_FRAMES: u8 = 6;

impl Cube {
    pub fn new(cfg: Config) -> Self {
        Cube {
            cfg,
            gravity: Gravity::new(),
            tracker: UpTracker::new(Heading::default()),
            controls: Controls::new(),
            view: View::new(),
            cache: BlockCache::new(),
            fallback: Fallback::new(),
            last_world: None,
            last_ms: None,
            scene: Scene::Other,
            other_run: 0,
        }
    }

    pub fn heading(&self) -> Heading {
        self.tracker.heading()
    }

    pub fn rolls(&self) -> u32 {
        self.tracker.rolls()
    }

    pub fn walking(&self) -> Option<crate::geom::Compass> {
        self.controls.walking()
    }

    /// Feed one IMU sample and the touch pads; returns the joypad.
    pub fn sense(&mut self, now_ms: u32, imu: &ImuSample, touch: u8) -> Buttons {
        let dt = self.last_ms.map_or(1.0 / 60.0, |t| {
            (now_ms.wrapping_sub(t) as f32 / 1000.0).clamp(0.0, 0.1)
        });
        self.last_ms = Some(now_ms);
        self.gravity.update(imu, dt);
        let up = self.gravity.up();
        if self.tracker.update(up, &self.cfg.up) {
            self.controls.rolled(now_ms, &self.cfg.controls);
        }
        let s = Sense {
            now_ms,
            up,
            accel_mg: imu.accel_mg,
            touch,
        };
        let heading = self.tracker.heading();
        self.controls.update(&s, &heading, &self.cfg.controls)
    }

    /// Draw all six faces for the frame that just finished.
    pub fn render<M: GbMem + ?Sized>(&mut self, f: &FrameIn<'_, M>, faces: &mut [FaceBuf; 6]) -> Report {
        let heading = self.tracker.heading();
        let layout = Layout::new(heading);
        let bottom = layout.face_with(Role::Bottom).index();
        faces[bottom].fill(0);

        let (map, cam, info) = if f.crystal {
            let map = MapInfo::read(f.mem);
            let cam = map.as_ref().and_then(|m| Camera::read(f.mem, m));
            let info = screen::classify(f.mem, map.as_ref(), cam.as_ref(), &mut self.cache);
            (map, cam, info)
        } else {
            (None, None, ScreenInfo::other())
        };
        self.fallback.observe(f.mem, f.crystal.then_some(&info), f.frame);

        // Hold the map through a frame or two of RAM that doesn't look like
        // it (see `HOLD_FRAMES`).
        self.other_run = if info.scene == Scene::Other {
            self.other_run.saturating_add(1)
        } else {
            0
        };
        let on_map = matches!(self.scene, Scene::Overworld | Scene::OverworldUi);
        if on_map && info.scene == Scene::Other && self.other_run < HOLD_FRAMES {
            // The canvas still holds the last map drawn.
            self.view.drape_onto(&layout, faces, 1 << bottom);
            faces[bottom].fill(0);
            return Report {
                scene: self.scene,
                drawn: Drawn::World,
                camera: cam,
                screen_on_canvas: self.last_world.map(|c| c.screen_on_canvas()),
                info,
                heading,
                reused: true,
            };
        }
        self.scene = info.scene;

        let mut report = Report {
            scene: info.scene,
            drawn: Drawn::World,
            camera: cam,
            screen_on_canvas: None,
            info,
            heading,
            reused: false,
        };
        let fog = self.cfg.fog_px;
        let overworld = matches!(info.scene, Scene::Overworld | Scene::OverworldUi);

        // The map, when there is one, for the overworld and for C's sides.
        let mut world_ready = false;
        if overworld && !(info.scene == Scene::OverworldUi && self.cfg.ui != UiStyle::Front) {
            if let (Some(map), Some(cam)) = (map, cam) {
                match self.cfg.wrap {
                    Wrap::World if crate::crystal::settled(f.mem) => {
                        world::draw(f.mem, &map, &cam, &mut self.cache, &mut self.view);
                        self.view.compute_light(fog);
                        self.last_world = Some(cam);
                        report.screen_on_canvas = Some(cam.screen_on_canvas());
                        report.drawn = Drawn::World;
                    }
                    Wrap::World => {
                        // Mid-update RAM: keep last frame's canvas rather
                        // than draw a torn one.
                        report.reused = true;
                        report.screen_on_canvas = self.last_world.map(|c| c.screen_on_canvas());
                        report.drawn = Drawn::World;
                    }
                    Wrap::Frame => {
                        self.view.from_frame(f.frame, f.palette, cam.centre);
                        self.view.compute_light(fog);
                        report.drawn = Drawn::Frame;
                    }
                }
                world_ready = true;
            }
        }

        let c_scene = info.scene == Scene::Other && f.crystal && self.cfg.ui == UiStyle::Front;
        let naming = if c_scene {
            screens::Naming::read(f.mem)
        } else {
            None
        };
        match info.scene {
            Scene::Overworld if world_ready => {
                self.view.drape_onto(&layout, faces, 1 << bottom);
            }
            Scene::OverworldUi if world_ready => {
                // C: map on five faces, the text on the front.
                self.view.drape_onto(&layout, faces, 1 << bottom);
                fallback::front_panel(f.mem, Some(&info), &layout, faces, &self.fallback);
                report.drawn = Drawn::Front;
            }
            Scene::Battle if self.cfg.ui == UiStyle::Front => {
                fallback::battle_crops(f.frame, f.palette, &layout, faces);
                if !fallback::front_panel(f.mem, None, &layout, faces, &self.fallback) {
                    let s = layout.face_with(Role::Side(crate::geom::Compass::South)).index();
                    faces[s].fill(0);
                }
                report.drawn = Drawn::Front;
            }
            _ if self.cfg.ui == UiStyle::Spread && info.scene != Scene::Overworld => {
                fallback::spread(f.frame, f.palette, &layout, faces);
                report.drawn = Drawn::Spread;
            }
            _ if !f.crystal && self.cfg.wrap == Wrap::Frame => {
                // Phase 2 for any game: fold the frame round a fixed spot.
                self.view.from_frame(f.frame, f.palette, self.cfg.default_centre);
                self.view.compute_light(fog);
                self.view.drape_onto(&layout, faces, 1 << bottom);
                report.drawn = Drawn::Frame;
            }
            Scene::Other if naming.is_some() => {
                if let Some(n) = naming {
                    n.draw(f.mem, &layout, faces);
                }
                report.drawn = Drawn::Naming;
            }
            Scene::Other
                if c_scene
                    && screens::still(
                        f.mem,
                        f.frame,
                        f.palette,
                        &mut self.view,
                        &layout,
                        faces,
                        &self.fallback,
                        fog,
                    ) =>
            {
                report.drawn = Drawn::Still;
            }
            _ => {
                let info_ref = f.crystal.then_some(&info);
                self.fallback.pan(
                    f.mem,
                    info_ref,
                    f.frame,
                    f.palette,
                    &mut self.view,
                    &layout,
                    faces,
                    fog,
                );
                report.drawn = Drawn::Pan;
            }
        }
        faces[bottom].fill(0);
        report
    }
}
