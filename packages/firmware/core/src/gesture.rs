//! Touch gestures: turns the touch mask into taps and holds (brief 3, 2.1–2.2).
//!
//! A capacitive face is touched every time someone picks the die up, so only
//! deliberate touches count:
//!
//! - **Grip.** Two or more faces touched at once is a hand around the die.
//!   It never taps or holds, and the touch stays dead until every finger is
//!   off.
//! - **Steady.** A touch counts only while the die is steady (the firmware
//!   decides what that means: at rest, or in the menu, not mid-tip). If the
//!   die moves under the finger, the touch is cancelled and stays dead until
//!   it's lifted, so a shake mid-hold never commits anything.
//! - **Tap.** Released before the hold ring appears ([`HOLD_RING_AFTER_MS`]).
//!   A tap is read-only: it may wake the die or show a label, never change
//!   game state.
//! - **Hold.** Held still on one face for longer than [`HOLD_MS`]. Letting go
//!   between the ring appearing and the hold firing cancels: nothing happens.
//!
//! ```text
//!            one face, steady                 > HOLD_MS
//!   Up ──────────────────────▶ Down ─────────────────────▶ Held ── lift ──▶ Up (Released)
//!    ▲                          │ lift ≤ ring: Tap
//!    │                          │ lift > ring: (cancel)
//!    │                          │ grip / unsteady
//!    └──────── lift ─────── Dead ◀──────────────────── (also from Held)
//! ```
//!
//! Like the other machines in this crate it is pure: the firmware feeds it
//! the mask, the time and whether the die is steady, and acts on the
//! [`Gesture`]s it returns.

use smokebomb_hal::Face;

/// How long a screen must be held to count as a hold (SIM_SPEC C3).
pub const HOLD_MS: u64 = 800;
/// The hold ring appears once a touch is clearly a hold, not a tap. A touch
/// let go before this is a tap.
pub const HOLD_RING_AFTER_MS: u64 = 220;
/// A tap counts as deliberate only on a die that had been resting this long
/// when the finger landed: picking the die up puts a finger on a face too.
pub const TAP_REST_MS: u64 = 400;

/// What the touch did this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A finger landed on a steady die (on one face or several). Read-only:
    /// it may wake the screens or bring back a dimmed result.
    Touched,
    /// A short touch on one face, let go before the hold ring.
    Tap {
        face: Face,
        /// The die had been resting for [`TAP_REST_MS`] when it began.
        deliberate: bool,
    },
    /// Held on one face for [`HOLD_MS`].
    Hold { face: Face },
    /// The finger lifted after a [`Gesture::Hold`].
    Released { face: Face },
}

/// One tick's input.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub now: u64,
    /// Touched faces, one bit per [`Face`] index.
    pub mask: u8,
    /// The die is steady enough for a touch to count.
    pub steady: bool,
    /// How long the die has been resting, as far as the firmware knows.
    pub rested_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Up,
    Down {
        face: Face,
        since: u64,
        deliberate: bool,
    },
    Held {
        face: Face,
    },
    /// Gripped, moved, or cancelled: ignored until every finger is off.
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct Gestures {
    state: State,
}

impl Default for Gestures {
    fn default() -> Self {
        Self::new()
    }
}

/// The one face in `mask`, if exactly one is touched.
fn single_face(mask: u8) -> Option<Face> {
    (mask.count_ones() == 1)
        .then(|| Face::from_index(mask.trailing_zeros() as usize))
        .flatten()
}

impl Gestures {
    pub const fn new() -> Self {
        Self { state: State::Up }
    }

    /// A finger is on the die (counted or not).
    pub fn touching(&self) -> bool {
        self.state != State::Up
    }

    /// The face being held toward a hold, and how far it has got (0–1),
    /// once the ring shows and until the hold fires.
    pub fn hold_progress(&self, now: u64) -> Option<(Face, f32)> {
        match self.state {
            State::Down { face, since, .. } => {
                let held = now.saturating_sub(since);
                (held > HOLD_RING_AFTER_MS).then(|| {
                    let p = (held - HOLD_RING_AFTER_MS) as f32 / (HOLD_MS - HOLD_RING_AFTER_MS) as f32;
                    (face, p.min(1.0))
                })
            }
            _ => None,
        }
    }

    /// Advance one tick.
    pub fn update(&mut self, i: Input) -> Option<Gesture> {
        let (next, out) = match self.state {
            State::Up if i.mask == 0 => (State::Up, None),
            // Landing on a moving die is a grab: dead until lifted.
            State::Up if !i.steady => (State::Dead, None),
            State::Up => {
                let next = match single_face(i.mask) {
                    Some(face) => State::Down {
                        face,
                        since: i.now,
                        deliberate: i.rested_ms >= TAP_REST_MS,
                    },
                    None => State::Dead,
                };
                (next, Some(Gesture::Touched))
            }
            State::Down {
                face,
                since,
                deliberate,
            } => {
                let held = i.now.saturating_sub(since);
                if i.mask == 0 {
                    let tap = (held <= HOLD_RING_AFTER_MS).then_some(Gesture::Tap { face, deliberate });
                    (State::Up, tap)
                } else if !i.steady || single_face(i.mask) != Some(face) {
                    (State::Dead, None)
                } else if held > HOLD_MS {
                    // At 60 Hz that's 49 frames, as in the mockup, whose
                    // float clock never quite reaches 0.8 after 48.
                    (State::Held { face }, Some(Gesture::Hold { face }))
                } else {
                    (self.state, None)
                }
            }
            State::Held { face } if i.mask == 0 => (State::Up, Some(Gesture::Released { face })),
            State::Held { face } if single_face(i.mask) != Some(face) => (State::Dead, None),
            State::Held { .. } => (self.state, None),
            State::Dead if i.mask == 0 => (State::Up, None),
            State::Dead => (State::Dead, None),
        };
        self.state = next;
        out
    }
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use super::*;

    const TICK: u64 = 16;
    const Z: u8 = 1 << Face::PosZ.index();
    const X: u8 = 1 << Face::PosX.index();

    /// Runs `mask` from `t0` for `ms`, steady unless `shake_at` is reached,
    /// and collects the gestures.
    fn run(g: &mut Gestures, t0: u64, ms: u64, mask: u8, unsteady_from: Option<u64>) -> Vec<Gesture> {
        let mut out = Vec::new();
        let mut t = t0;
        while t < t0 + ms {
            let steady = unsteady_from.is_none_or(|u| t < u);
            if let Some(e) = g.update(Input {
                now: t,
                mask,
                steady,
                rested_ms: 1_000 + t,
            }) {
                out.push(e);
            }
            t += TICK;
        }
        out
    }

    fn lift(g: &mut Gestures, t: u64) -> Option<Gesture> {
        g.update(Input {
            now: t,
            mask: 0,
            steady: true,
            rested_ms: 1_000,
        })
    }

    #[test]
    fn a_short_touch_is_a_deliberate_tap() {
        let mut g = Gestures::new();
        assert_eq!(run(&mut g, 0, 150, Z, None), [Gesture::Touched]);
        assert_eq!(
            lift(&mut g, 160),
            Some(Gesture::Tap {
                face: Face::PosZ,
                deliberate: true
            })
        );
        assert!(!g.touching());
    }

    #[test]
    fn a_tap_on_a_die_just_set_down_is_not_deliberate() {
        let mut g = Gestures::new();
        g.update(Input {
            now: 0,
            mask: Z,
            steady: true,
            rested_ms: TAP_REST_MS - 1,
        });
        assert_eq!(
            lift(&mut g, 50),
            Some(Gesture::Tap {
                face: Face::PosZ,
                deliberate: false
            })
        );
    }

    #[test]
    fn a_long_touch_holds_once_then_releases() {
        let mut g = Gestures::new();
        let events = run(&mut g, 0, 2_000, Z, None);
        assert_eq!(events, [Gesture::Touched, Gesture::Hold { face: Face::PosZ }]);
        assert_eq!(lift(&mut g, 2_000), Some(Gesture::Released { face: Face::PosZ }));
    }

    #[test]
    fn letting_go_after_the_ring_shows_cancels() {
        // Brief 3, test 5: let go at 0.7 s and nothing happens.
        let mut g = Gestures::new();
        run(&mut g, 0, 700, Z, None);
        assert!(g.hold_progress(690).is_some(), "the ring is showing");
        assert_eq!(lift(&mut g, 700), None);
        assert!(!g.touching());
    }

    #[test]
    fn the_ring_shows_after_the_tap_window_and_fills_by_the_hold() {
        let mut g = Gestures::new();
        run(&mut g, 0, 200, Z, None);
        assert_eq!(g.hold_progress(HOLD_RING_AFTER_MS), None);
        let (face, p) = g.hold_progress(HOLD_RING_AFTER_MS + 290).unwrap();
        assert_eq!(face, Face::PosZ);
        assert!((p - 0.5).abs() < 0.01);
        assert_eq!(g.hold_progress(HOLD_MS + 100).unwrap().1, 1.0);
    }

    #[test]
    fn a_grip_never_taps_or_holds() {
        // Brief 3, test 4: two faces for 2 s commits nothing.
        let mut g = Gestures::new();
        let events = run(&mut g, 0, 2_000, Z | X, None);
        assert_eq!(events, [Gesture::Touched]);
        assert_eq!(g.hold_progress(1_500), None);
        assert_eq!(lift(&mut g, 2_000), None);
    }

    #[test]
    fn a_second_face_turns_a_hold_into_a_grip() {
        let mut g = Gestures::new();
        run(&mut g, 0, 400, Z, None);
        assert!(run(&mut g, 400, 1_600, Z | X, None).is_empty());
        // Letting the second finger go doesn't bring the hold back.
        assert!(run(&mut g, 2_000, 1_000, Z, None).is_empty());
        assert_eq!(lift(&mut g, 3_000), None);
    }

    #[test]
    fn moving_the_die_mid_hold_cancels_until_lifted() {
        // Brief 3, test 7: a shake during the hold commits nothing, even if
        // the finger stays on once the die is put down.
        let mut g = Gestures::new();
        let events = run(&mut g, 0, 600, Z, Some(500));
        assert_eq!(events, [Gesture::Touched]);
        assert_eq!(g.hold_progress(600), None);
        assert!(run(&mut g, 600, 2_000, Z, None).is_empty());
        assert_eq!(lift(&mut g, 2_600), None);
        // A fresh touch works again.
        assert_eq!(run(&mut g, 3_000, 1_000, Z, None).len(), 2);
    }

    #[test]
    fn a_finger_that_lands_on_a_moving_die_is_ignored_until_lifted() {
        let mut g = Gestures::new();
        assert!(run(&mut g, 0, 300, Z, Some(0)).is_empty());
        assert!(run(&mut g, 300, 2_000, Z, None).is_empty());
        assert_eq!(lift(&mut g, 2_300), None);
    }

    #[test]
    fn sliding_to_another_face_cancels() {
        let mut g = Gestures::new();
        run(&mut g, 0, 300, Z, None);
        assert!(run(&mut g, 300, 1_500, X, None).is_empty());
        assert_eq!(lift(&mut g, 1_800), None);
    }
}
