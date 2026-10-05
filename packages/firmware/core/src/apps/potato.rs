//! Hot Potato as an app: the game is [`crate::potato`]; this lights it on a
//! shake, resets it on a tap, and turns its commands into effects.

use smokebomb_hal::{HalResult, HapticEffect};

use super::{Action, App, Ctx, Effect, Effects, Kind, MotionUse, SmokeOp, Tap};
use crate::potato::{self as game, Potato, PotatoCommand, PotatoCommands};
use crate::state::Mode;

/// How much of a full cloud a lit fuse's smoke may reach, at the start and
/// at full heat: enough to build, little enough to read "PASS IT" through.
const SMOLDER_MIN: f32 = 0.08;
const SMOLDER_MAX: f32 = 0.3;

/// The game's commands as effects.
pub fn commands(cmds: PotatoCommands) -> Effects {
    let mut out = Effects::new();
    for cmd in cmds {
        push_command(&mut out, cmd);
    }
    out
}

fn push_command(out: &mut Effects, cmd: PotatoCommand) {
    let (a, b) = match cmd {
        PotatoCommand::Ignite => (
            Effect::Haptic(HapticEffect::Tick),
            Some(Effect::Smoke(SmokeOp::Smolder(SMOLDER_MIN))),
        ),
        PotatoCommand::Tick => (Effect::Haptic(HapticEffect::Tick), None),
        PotatoCommand::Boom => (
            Effect::Haptic(HapticEffect::Buzz),
            Some(Effect::Smoke(SmokeOp::Burst)),
        ),
        PotatoCommand::Clear => (Effect::Smoke(SmokeOp::Clear), None),
    };
    let _ = out.push(a);
    if let Some(b) = b {
        let _ = out.push(b);
    }
}

impl App for Potato {
    fn kind(&self) -> Kind {
        Kind {
            motion: MotionUse::Shake,
            scored: false,
        }
    }

    /// A shake lights an idle die: the fuse is drawn at random within the
    /// setting.
    fn shaken(&mut self, cx: &mut Ctx) -> HalResult<Effects> {
        let mut out = Effects::new();
        if !self.is_idle() {
            return Ok(out);
        }
        let range = cx.settings.fuse.range_ms();
        let fuse = game::fuse_from(cx.rng.next_u32()?, range);
        let _ = out.push(Effect::GameStarted);
        for cmd in self.light(cx.now, fuse, range.1) {
            push_command(&mut out, cmd);
        }
        Ok(out)
    }

    fn tapped(&self, _tap: Tap, cx: &Ctx) -> Option<Action> {
        (!matches!(cx.mode, Mode::Menu | Mode::Nest | Mode::Off)).then_some(Action::Reset)
    }

    fn commit(&mut self, action: Action, cx: &mut Ctx) -> Effects {
        match action {
            Action::Reset => commands(self.tap(cx.now)),
            _ => Effects::new(),
        }
    }

    /// Tick the fuse, and keep the smoke building with the heat.
    fn tick(&mut self, cx: &mut Ctx) -> Effects {
        let mut out = commands(Potato::tick(self, cx.now));
        if self.is_lit() {
            let heat = self.heat(cx.now);
            let _ = out.push(Effect::Smoke(SmokeOp::Smolder(
                SMOLDER_MIN + (SMOLDER_MAX - SMOLDER_MIN) * heat,
            )));
        }
        out
    }

    fn busy(&self) -> bool {
        !self.is_idle()
    }

    /// No opening the menu mid-round.
    fn blocks_hold(&self) -> bool {
        self.is_lit()
    }
}
