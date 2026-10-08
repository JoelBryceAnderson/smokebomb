//! The joypad, in walnut-cgb's bit order (`JOYPAD_*`), active high.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons(u8);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    pub const A: Buttons = Buttons(0x01);
    pub const B: Buttons = Buttons(0x02);
    pub const SELECT: Buttons = Buttons(0x04);
    pub const START: Buttons = Buttons(0x08);
    pub const RIGHT: Buttons = Buttons(0x10);
    pub const LEFT: Buttons = Buttons(0x20);
    pub const UP: Buttons = Buttons(0x40);
    pub const DOWN: Buttons = Buttons(0x80);
    pub const DPAD: Buttons = Buttons(0xF0);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(b: u8) -> Buttons {
        Buttons(b)
    }

    pub const fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Buttons) -> Buttons {
        Buttons(self.0 | other.0)
    }

    pub const fn without(self, other: Buttons) -> Buttons {
        Buttons(self.0 & !other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, rhs: Buttons) -> Buttons {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Buttons {
    fn bitor_assign(&mut self, rhs: Buttons) {
        *self = self.union(rhs);
    }
}
