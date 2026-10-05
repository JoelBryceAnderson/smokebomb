//! Hot Potato as an app: the game is [`crate::potato`]; this lights it on a
//! shake and turns its commands into effects. Once it has gone off, a shake
//! starts the next round (after [`RESTART_GUARD_MS`], so passing the die as
//! it goes off doesn't), or it resets by itself. A hold opens the menu,
//! except while the fuse is lit.

use smokebomb_hal::{HalResult, HapticEffect};

use super::{App, Ctx, Effect, Effects, Kind, MotionUse, SmokeOp};
use crate::potato::{self as game, Potato, PotatoCommand, PotatoCommands};

/// After it goes off, shakes are ignored this long.
pub const RESTART_GUARD_MS: u64 = 1_500;

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

    /// A shake lights an idle die, or a spent one once the guard is past:
    /// the fuse is drawn at random within the setting.
    fn shaken(&mut self, cx: &mut Ctx) -> HalResult<Effects> {
        let mut out = Effects::new();
        if self.boomed_for(cx.now).is_some_and(|t| t >= RESTART_GUARD_MS) {
            for cmd in self.reset() {
                push_command(&mut out, cmd);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Settings;
    use crate::state::Mode;
    use smokebomb_hal::Rng;

    struct Words(u32);
    impl Rng for Words {
        fn fill_bytes(&mut self, buf: &mut [u8]) -> HalResult<()> {
            buf.copy_from_slice(&self.0.to_le_bytes()[..buf.len()]);
            Ok(())
        }
    }

    fn shake(p: &mut Potato, now: u64) -> Effects {
        let mut settings = Settings::default();
        let mut rng = Words(0);
        let mut cx = Ctx {
            now,
            mode: Mode::Idle,
            label_up: false,
            settings: &mut settings,
            rng: &mut rng,
        };
        p.shaken(&mut cx).unwrap()
    }

    #[test]
    fn a_shake_restarts_a_spent_die_only_after_the_guard() {
        let mut p = Potato::new();
        assert!(shake(&mut p, 0).contains(&Effect::GameStarted));
        let fuse = match p.state() {
            game::PotatoState::Lit { fuse_ms, .. } => *fuse_ms as u64,
            s => panic!("{s:?}"),
        };
        assert!(shake(&mut p, 10).is_empty(), "already lit");
        Potato::tick(&mut p, fuse);
        assert!(p.boomed_for(fuse).is_some());
        assert!(shake(&mut p, fuse + RESTART_GUARD_MS - 1).is_empty(), "too soon");
        let fx = shake(&mut p, fuse + RESTART_GUARD_MS);
        assert_eq!(fx[0], Effect::Smoke(SmokeOp::Clear));
        assert!(fx.contains(&Effect::GameStarted));
        assert!(p.is_lit());
    }
}
