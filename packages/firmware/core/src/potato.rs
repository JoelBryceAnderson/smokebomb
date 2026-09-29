//! Hot Potato.
//!
//! Shake the die to light the fuse, then pass it around. It ticks against a
//! fuse the die picked at random, so nobody can count it down: the ticks
//! and the glow speed up with the time spent against the longest fuse the
//! setting allows, and whoever holds it when the fuse runs out is out.
//! Tapping a spent die, or leaving it for [`BOOM_MS`], resets it.
//!
//! ```text
//!   Idle ── shake ──▶ Lit ── fuse runs out ──▶ Boom ── tap / BOOM_MS ──▶ Idle
//!    ▲                 │                          │
//!    └──── reset (menu, docking) ◀────────────────┘
//! ```
//!
//! Like [`crate::state::StateMachine`] the machine is pure: the firmware
//! draws the fuse from the RNG, passes the time in, and executes the
//! [`PotatoCommand`]s it returns. The roll state machine stays out of a game that
//! doesn't roll (see `StateMachine::set_rolls`); it still owns the menu and
//! the Nest.

use heapless::Vec;

/// What the firmware does for the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PotatoCommand {
    /// The fuse is lit: a tick, and smoke starts building.
    Ignite,
    /// A tick of the fuse.
    Tick,
    /// It went off: a buzz and a cloud.
    Boom,
    /// Back to idle: the smoke goes.
    Clear,
}

pub type PotatoCommands = Vec<PotatoCommand, 2>;

/// How long the spent die shows BOOM before it resets by itself.
pub const BOOM_MS: u64 = 6_000;
/// A tap dismisses BOOM only after this long, so the shake that ends a
/// round doesn't.
pub const BOOM_TAP_GUARD_MS: u64 = 800;
/// Gap between ticks when the fuse is lit, and when it's about to go.
/// How long the glow takes to fade after a tick.
const PULSE_MS: f32 = 260.0;
const SLOWEST_TICK_MS: f32 = 900.0;
const FASTEST_TICK_MS: f32 = 110.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PotatoState {
    Idle,
    Lit {
        since_ms: u64,
        /// When it goes off, after `since_ms`.
        fuse_ms: u32,
        /// The longest this setting allows: what the ticks pace against.
        span_ms: u32,
        last_tick_ms: u64,
        next_tick_ms: u64,
    },
    Boom {
        since_ms: u64,
    },
}

pub struct Potato {
    state: PotatoState,
}

impl Default for Potato {
    fn default() -> Self {
        Self::new()
    }
}

impl Potato {
    pub const fn new() -> Self {
        Self {
            state: PotatoState::Idle,
        }
    }

    pub fn state(&self) -> &PotatoState {
        &self.state
    }

    pub fn is_idle(&self) -> bool {
        self.state == PotatoState::Idle
    }

    pub fn is_lit(&self) -> bool {
        matches!(self.state, PotatoState::Lit { .. })
    }

    /// A shake lit the fuse. Only an idle die lights.
    pub fn light(&mut self, now_ms: u64, fuse_ms: u32, span_ms: u32) -> PotatoCommands {
        let mut out = PotatoCommands::new();
        if self.is_idle() {
            let _ = out.push(PotatoCommand::Ignite);
            self.state = PotatoState::Lit {
                since_ms: now_ms,
                fuse_ms,
                span_ms: span_ms.max(fuse_ms).max(1),
                last_tick_ms: now_ms,
                next_tick_ms: now_ms + SLOWEST_TICK_MS as u64,
            };
        }
        out
    }

    /// A tap on a spent die resets it.
    pub fn tap(&mut self, now_ms: u64) -> PotatoCommands {
        match self.state {
            PotatoState::Boom { since_ms } if now_ms - since_ms >= BOOM_TAP_GUARD_MS => self.reset(),
            _ => PotatoCommands::new(),
        }
    }

    /// Back to idle, from wherever it was.
    pub fn reset(&mut self) -> PotatoCommands {
        let mut out = PotatoCommands::new();
        if !self.is_idle() {
            let _ = out.push(PotatoCommand::Clear);
            self.state = PotatoState::Idle;
        }
        out
    }

    /// Advance the fuse: tick, go off, or time out a spent die.
    pub fn tick(&mut self, now_ms: u64) -> PotatoCommands {
        let mut out = PotatoCommands::new();
        match self.state {
            PotatoState::Lit {
                since_ms,
                fuse_ms,
                next_tick_ms,
                ..
            } => {
                if now_ms - since_ms >= fuse_ms as u64 {
                    let _ = out.push(PotatoCommand::Boom);
                    self.state = PotatoState::Boom { since_ms: now_ms };
                } else if now_ms >= next_tick_ms {
                    let _ = out.push(PotatoCommand::Tick);
                    let gap = tick_gap_ms(self.heat(now_ms));
                    if let PotatoState::Lit {
                        last_tick_ms,
                        next_tick_ms,
                        ..
                    } = &mut self.state
                    {
                        *last_tick_ms = now_ms;
                        *next_tick_ms = now_ms + gap as u64;
                    }
                }
            }
            PotatoState::Boom { since_ms } if now_ms - since_ms >= BOOM_MS => {
                return self.reset();
            }
            _ => {}
        }
        out
    }

    /// How hot the die feels, 0–1: time lit against the longest fuse the
    /// setting allows. It never gives the real fuse away.
    pub fn heat(&self, now_ms: u64) -> f32 {
        match self.state {
            PotatoState::Lit {
                since_ms, span_ms, ..
            } => ((now_ms - since_ms) as f32 / span_ms as f32).clamp(0.0, 1.0),
            PotatoState::Boom { .. } => 1.0,
            PotatoState::Idle => 0.0,
        }
    }

    /// A flash that peaks on each tick and fades over [`PULSE_MS`], 0–1.
    pub fn pulse(&self, now_ms: u64) -> f32 {
        match self.state {
            PotatoState::Lit { last_tick_ms, .. } => {
                (1.0 - now_ms.saturating_sub(last_tick_ms) as f32 / PULSE_MS).max(0.0)
            }
            _ => 0.0,
        }
    }

    /// Milliseconds since it went off, if it has.
    pub fn boomed_for(&self, now_ms: u64) -> Option<u64> {
        match self.state {
            PotatoState::Boom { since_ms } => Some(now_ms - since_ms),
            _ => None,
        }
    }
}

/// The gap to the next tick at `heat` (0–1): slow at first, then racing.
pub fn tick_gap_ms(heat: f32) -> f32 {
    let h = heat.clamp(0.0, 1.0);
    SLOWEST_TICK_MS + (FASTEST_TICK_MS - SLOWEST_TICK_MS) * h * h
}

/// A fuse in `lo..=hi` ms from a uniform `r` (multiply-shift, so no modulo
/// bias worth speaking of at millisecond resolution).
pub fn fuse_from(r: u32, (lo, hi): (u32, u32)) -> u32 {
    let width = (hi.saturating_sub(lo) as u64) + 1;
    lo + ((r as u64 * width) >> 32) as u32
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn has(cmds: &PotatoCommands, c: PotatoCommand) -> bool {
        cmds.contains(&c)
    }

    #[test]
    fn a_shake_lights_an_idle_die_and_only_an_idle_one() {
        let mut p = Potato::new();
        let cmds = p.light(1_000, 15_000, 20_000);
        assert!(has(&cmds, PotatoCommand::Ignite));
        assert!(p.is_lit());
        assert!(p.light(2_000, 5_000, 20_000).is_empty(), "already lit");
        assert!(matches!(p.state(), PotatoState::Lit { fuse_ms: 15_000, .. }));
    }

    #[test]
    fn it_ticks_faster_as_it_heats_up_and_goes_off_at_the_fuse() {
        let mut p = Potato::new();
        p.light(0, 10_000, 20_000);
        let mut ticks = std::vec::Vec::new();
        let mut boomed_at = None;
        for t in 0..=10_500u64 {
            let cmds = p.tick(t);
            if has(&cmds, PotatoCommand::Tick) {
                ticks.push(t);
            }
            if has(&cmds, PotatoCommand::Boom) {
                boomed_at = Some(t);
                break;
            }
        }
        assert_eq!(boomed_at, Some(10_000));
        assert!(matches!(p.state(), PotatoState::Boom { since_ms: 10_000 }));
        let gaps: std::vec::Vec<u64> = ticks.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.len() > 8);
        assert!(gaps.first().unwrap() > gaps.last().unwrap(), "{gaps:?}");
        assert!(
            gaps.windows(2).all(|w| w[1] <= w[0] + 1),
            "never slows down: {gaps:?}"
        );
    }

    #[test]
    fn heat_paces_against_the_longest_fuse_not_the_real_one() {
        let mut p = Potato::new();
        p.light(0, 10_000, 40_000);
        assert_eq!(p.heat(0), 0.0);
        assert!(
            (p.heat(10_000) - 0.25).abs() < 1e-6,
            "a quarter at the fuse of a 40 s span"
        );
    }

    #[test]
    fn boom_shows_for_a_while_then_resets() {
        let mut p = Potato::new();
        p.light(0, 1_000, 1_000);
        p.tick(1_000);
        assert_eq!(p.boomed_for(1_500), Some(500));
        assert!(p.tick(1_000 + BOOM_MS - 1).is_empty());
        let cmds = p.tick(1_000 + BOOM_MS);
        assert!(has(&cmds, PotatoCommand::Clear));
        assert!(p.is_idle());
    }

    #[test]
    fn a_tap_resets_a_spent_die_but_not_at_once() {
        let mut p = Potato::new();
        p.light(0, 1_000, 1_000);
        assert!(p.tap(500).is_empty(), "lit dice ignore taps");
        p.tick(1_000);
        assert!(p.tap(1_000 + BOOM_TAP_GUARD_MS - 1).is_empty(), "too soon");
        assert!(has(&p.tap(1_000 + BOOM_TAP_GUARD_MS), PotatoCommand::Clear));
        assert!(p.is_idle());
    }

    #[test]
    fn reset_stops_a_lit_die_and_is_quiet_when_idle() {
        let mut p = Potato::new();
        assert!(p.reset().is_empty());
        p.light(0, 5_000, 5_000);
        assert!(has(&p.reset(), PotatoCommand::Clear));
        assert!(p.is_idle());
        assert_eq!(p.heat(100), 0.0);
    }

    #[test]
    fn fuses_stay_in_range() {
        let range = (10_000, 20_000);
        assert_eq!(fuse_from(0, range), 10_000);
        assert_eq!(fuse_from(u32::MAX, range), 20_000);
        for r in [1u32, 1 << 20, 1 << 31, u32::MAX - 1] {
            let f = fuse_from(r, range);
            assert!((10_000..=20_000).contains(&f), "{f}");
        }
        assert_eq!(fuse_from(123, (5, 5)), 5);
    }

    #[test]
    fn the_glow_flashes_on_each_tick() {
        let mut p = Potato::new();
        p.light(0, 10_000, 10_000);
        assert!((p.pulse(0) - 1.0).abs() < 1e-6);
        assert!(p.pulse(130) < p.pulse(0));
        assert_eq!(p.pulse(400), 0.0);
        p.tick(900);
        assert!((p.pulse(900) - 1.0).abs() < 1e-6);
        assert_eq!(Potato::new().pulse(0), 0.0);
    }

    #[test]
    fn tick_gaps_shrink() {
        assert!(tick_gap_ms(0.0) > tick_gap_ms(0.5));
        assert!(tick_gap_ms(0.5) > tick_gap_ms(1.0));
        assert!((tick_gap_ms(2.0) - tick_gap_ms(1.0)).abs() < 1e-6);
    }
}
