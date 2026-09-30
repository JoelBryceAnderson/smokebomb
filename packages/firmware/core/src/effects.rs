//! What plays over the faces: the smoke (sugar crystals, coins), or in Pig
//! Toss the pigs. The two never show together, so they share one block of
//! memory, and switching modes builds the one needed in place.

use core::mem::{ManuallyDrop, MaybeUninit};
use core::ptr::addr_of_mut;

use smokebomb_hal::AssetStore;

use crate::pigfx::Canvas;
use crate::smoke::{Smoke, SmokeRng};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Smoke,
    Pigs,
}

/// Room for one or the other. Neither needs dropping: both are plain data.
union Room {
    smoke: ManuallyDrop<Smoke>,
    pigs: ManuallyDrop<Canvas>,
}

pub struct Effects {
    kind: Kind,
    room: Room,
    /// Where the pack keeps the smoke's sprites, to load them again.
    sprites: Option<u32>,
    /// The smoke's random generator while the pigs have the room, so the
    /// smoke picks up where it left off.
    parked: SmokeRng,
}

impl Effects {
    /// Start with the smoke, built in `slot`.
    pub fn init<'a, A: AssetStore>(
        slot: &'a mut MaybeUninit<Self>,
        assets: &mut A,
        sprites: Option<u32>,
        seed: u32,
    ) -> &'a mut Self {
        let p = slot.as_mut_ptr();
        // SAFETY: each field is written before anything reads it: the small
        // ones directly, the room through `Smoke::init`, which fills it in
        // place (`ManuallyDrop<Smoke>` has `Smoke`'s layout, as does
        // `MaybeUninit<Smoke>`).
        unsafe {
            addr_of_mut!((*p).kind).write(Kind::Smoke);
            addr_of_mut!((*p).sprites).write(sprites);
            addr_of_mut!((*p).parked).write(SmokeRng::new(seed));
            let room = addr_of_mut!((*p).room.smoke).cast::<MaybeUninit<Smoke>>();
            Smoke::init(&mut *room, assets, sprites, SmokeRng::new(seed));
            slot.assume_init_mut()
        }
    }

    /// The smoke, unless the pigs have the room.
    pub fn smoke(&mut self) -> Option<&mut Smoke> {
        // SAFETY: `kind` says which field of the room was built last.
        (self.kind == Kind::Smoke).then(|| unsafe { &mut *self.room.smoke })
    }

    /// The smoke, to look at.
    pub fn smoke_ref(&self) -> Option<&Smoke> {
        // SAFETY: as in `smoke`.
        (self.kind == Kind::Smoke).then(|| unsafe { &*self.room.smoke })
    }

    /// The pigs, unless the smoke has the room.
    pub fn pigs(&mut self) -> Option<&mut Canvas> {
        // SAFETY: as in `smoke`.
        (self.kind == Kind::Pigs).then(|| unsafe { &mut *self.room.pigs })
    }

    /// Give the room to the pigs. Whatever smoke there was goes.
    pub fn use_pigs(&mut self) -> &mut Canvas {
        if self.kind == Kind::Smoke {
            // SAFETY: the room holds smoke; it is read once, then the room
            // is rebuilt as a canvas in place and `kind` follows.
            unsafe {
                self.parked = self.room.smoke.rng();
                Canvas::init(&mut *addr_of_mut!(self.room.pigs).cast::<MaybeUninit<Canvas>>());
            }
            self.kind = Kind::Pigs;
        }
        self.pigs().expect("the pigs have the room")
    }

    /// Give the room to the smoke, loading its sprites again. It starts
    /// clear, with its settings at their defaults: set them after.
    pub fn use_smoke<A: AssetStore>(&mut self, assets: &mut A) -> &mut Smoke {
        if self.kind == Kind::Pigs {
            // SAFETY: the room holds a canvas, which needs no drop; it is
            // rebuilt as smoke in place and `kind` follows.
            unsafe {
                let room = addr_of_mut!(self.room.smoke).cast::<MaybeUninit<Smoke>>();
                Smoke::init(&mut *room, assets, self.sprites, self.parked);
            }
            self.kind = Kind::Smoke;
        }
        self.smoke().expect("the smoke has the room")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smokebomb_hal::{HalError, HalResult};

    /// A pack with nothing in it.
    struct NoAssets;

    impl AssetStore for NoAssets {
        fn capacity(&self) -> u32 {
            0
        }
        fn read(&mut self, _: u32, _: &mut [u8]) -> HalResult<()> {
            Err(HalError::NotReady)
        }
    }

    fn effects() -> Effects {
        let mut slot = MaybeUninit::uninit();
        Effects::init(&mut slot, &mut NoAssets, None, 7);
        // SAFETY: `init` wrote it.
        unsafe { slot.assume_init() }
    }

    #[test]
    fn the_room_goes_to_one_or_the_other() {
        let mut fx = effects();
        assert!(fx.smoke().is_some() && fx.pigs().is_none());
        fx.use_pigs();
        assert!(fx.smoke().is_none() && fx.smoke_ref().is_none() && fx.pigs().is_some());
        fx.use_smoke(&mut NoAssets);
        assert!(fx.smoke().is_some() && fx.pigs().is_none());
    }

    #[test]
    fn it_costs_the_bigger_of_the_two() {
        let (smoke, pigs) = (size_of::<Smoke>(), size_of::<Canvas>());
        assert!(size_of::<Effects>() <= smoke.max(pigs) + 32);
    }

    #[test]
    fn the_smoke_picks_up_its_random_sequence_where_it_left_off() {
        let mut fx = effects();
        let before = fx.smoke().unwrap().rng();
        fx.use_pigs();
        fx.use_pigs();
        let after = fx.use_smoke(&mut NoAssets).rng();
        assert_eq!(before, after);
    }

    #[test]
    fn smoke_comes_back_clear() {
        let mut fx = effects();
        let smoke = fx.smoke().unwrap();
        smoke.throw();
        assert!(!smoke.is_empty());
        fx.use_pigs();
        assert!(fx.use_smoke(&mut NoAssets).is_empty());
    }
}
