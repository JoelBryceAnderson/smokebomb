//! The phone link: the app's BLE messages to and from the die on the phone.
//!
//! On hardware the app talks to the die over BLE with the
//! `smokebomb_shared::protocol` messages. The die in the AR viewer is on the
//! same phone, so the app passes them straight across, as JSON text in
//! serde's default shape, the same as the desktop simulator's `/phone`
//! WebSocket (`packages/simulator/server/src/phone.rs`): `"GetInventory"`,
//! `{"SetEnabledModes":13}`, `{"Inventory":{"licensed":15,"enabled":13,"active":"Dice"}}`.
//! The app's link for either is the same code but for the transport.
//!
//! The die answers each message as it arrives (between ticks, as the die's
//! main loop drains BLE writes), and after each tick reports a new roll or a
//! change of mode made on the die itself.

use std::collections::VecDeque;

use smokebomb_hal::SecureElement;
use smokebomb_hal_simulator::SimSecureElement;
use smokebomb_shared::protocol::{DieToPhone, Inventory, PhoneToDie};
use smokebomb_shared::{LicensedItem, SignedRoll};

/// How many rolls the die keeps for `SyncHistory`, as the desktop simulator
/// does (the die will keep its recent rolls in flash).
pub const HISTORY_LEN: usize = 500;

/// What the die does when a message arrives: the firmware calls it needs.
pub trait PhoneDie {
    fn inventory(&self) -> Inventory;
    fn set_enabled_modes(&mut self, modes: smokebomb_shared::ModeSet);
    fn unlock_mode(&mut self, mode: smokebomb_shared::ModeId);
    fn set_die(&mut self, kind: smokebomb_shared::DieKind, count: u8);
    fn last_roll(&self) -> Option<&SignedRoll>;
    fn battery_percent(&self) -> u8;
}

/// The die's side of the link: whether a phone is connected, what's waiting
/// for it, and the rolls the die keeps.
#[derive(Debug, Default)]
pub struct PhoneLink {
    connected: bool,
    outbox: VecDeque<String>,
    history: VecDeque<SignedRoll>,
    last_counter: Option<u32>,
    last_inventory: Option<Inventory>,
}

impl PhoneLink {
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// A phone connects (the die greets it as on BLE) or goes.
    pub fn set_connected<D: PhoneDie>(&mut self, die: &D, connected: bool) {
        self.outbox.clear();
        self.connected = connected;
        if connected {
            self.send(&hello(die));
            self.send(&DieToPhone::Inventory(die.inventory()));
        }
        self.last_inventory = Some(die.inventory());
    }

    /// One message from the phone, as JSON. False if it isn't one.
    pub fn receive<D: PhoneDie>(&mut self, die: &mut D, json: &str) -> bool {
        let Ok(msg) = serde_json::from_str::<PhoneToDie>(json) else {
            return false;
        };
        if !self.connected {
            return true;
        }
        match msg {
            PhoneToDie::GetInventory => {}
            PhoneToDie::SetEnabledModes(modes) => die.set_enabled_modes(modes),
            PhoneToDie::InstallLicense(license) => match license.item {
                // Trusted, as on the desktop simulator; the die will check
                // the store's signature (docs/STORE.md).
                LicensedItem::Mode(mode) => die.unlock_mode(mode),
                LicensedItem::Theme(_) => {}
            },
            PhoneToDie::SetDie { kind, count } => die.set_die(kind, count),
            PhoneToDie::GetPublicKey => {
                if let Ok(key) = SimSecureElement::new().public_key() {
                    self.send(&DieToPhone::PublicKey(key));
                }
            }
            PhoneToDie::SyncHistory { since_counter } => {
                let rolls: Vec<_> = self
                    .history
                    .iter()
                    .filter(|r| r.record.counter >= since_counter)
                    .cloned()
                    .collect();
                for r in rolls {
                    self.send(&DieToPhone::HistoryItem(Some(r)));
                }
                self.send(&DieToPhone::HistoryItem(None));
                return true;
            }
            // Not on the simulated die yet, as on the desktop simulator.
            PhoneToDie::SetOwnerName(_) | PhoneToDie::JoinSession(_) | PhoneToDie::LeaveSession => {}
        }
        // Every setup change is answered with the inventory, even when
        // nothing changed, so the app always hears back.
        let inventory = die.inventory();
        self.last_inventory = Some(inventory);
        self.send(&DieToPhone::Inventory(inventory));
        true
    }

    /// After a tick: keep a new roll, and tell the phone about it and about
    /// a mode picked on the die.
    pub fn after_tick<D: PhoneDie>(&mut self, die: &D) {
        if let Some(roll) = die
            .last_roll()
            .filter(|r| Some(r.record.counter) != self.last_counter)
        {
            self.last_counter = Some(roll.record.counter);
            if self.history.len() == HISTORY_LEN {
                self.history.pop_front();
            }
            self.history.push_back(roll.clone());
            self.send(&DieToPhone::Roll(roll.clone()));
        }
        let inventory = die.inventory();
        if self.last_inventory != Some(inventory) {
            self.last_inventory = Some(inventory);
            self.send(&DieToPhone::Inventory(inventory));
        }
    }

    /// The next message for the phone, as JSON.
    pub fn next(&self) -> Option<&str> {
        self.outbox.front().map(String::as_str)
    }

    pub fn pop(&mut self) -> Option<String> {
        self.outbox.pop_front()
    }

    fn send(&mut self, msg: &DieToPhone) {
        if self.connected {
            self.outbox
                .push_back(serde_json::to_string(msg).expect("DieToPhone serialises"));
        }
    }
}

fn hello<D: PhoneDie>(die: &D) -> DieToPhone {
    let v = |i: usize| {
        env!("CARGO_PKG_VERSION")
            .split('.')
            .nth(i)
            .and_then(|p| p.parse().ok())
            .unwrap_or(0)
    };
    DieToPhone::Hello {
        firmware_version: (v(0), v(1), v(2)),
        battery_percent: die.battery_percent(),
    }
}
