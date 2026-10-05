# App framework (planned)

Status: agreed with Joel, 5 Oct 2026. Steps 1 to 5 are built; the 1 m AR
check is left. It applies `SMOKEBOMB_SIM_UPDATE_BRIEF_3` (text sizes and
deliberate gestures).

## Why

Each game is a special case in `Firmware` (`lib.rs`): branches in the main
loop for Potato's shake and tap, Pig Toss banking and Pot's bills, by-hand
rendering of each game's scenes, and `StateMachine::set_rolls(false)` so
Potato can opt out of rolling. The menu's `Draft` mixes mode choice, each
game's options, Pig Toss setup and End game, and the global Settings list.
Brief 3 touches all of it.

## Model: a platform that hosts apps

**The platform** owns boot, Nest, off and sleep, the roll flow (shake, air,
settle), the gesture arbiter, the hold ring and its action preview, the menu
and adjust mechanics, and the tier fonts.

**Apps** own their rules, each a pure state machine:

| App | Motion | Scored | Session | Takes over the die |
|---|---|---|---|---|
| Dice | rolls | no | none | yes |
| Pot | rolls | no | none (bills are options) | yes |
| Hot Potato | shake triggers | no | one round | yes |
| Pig Toss | rolls | yes | persistent | yes |
| Settings | none | no | none | no: leaving it returns to the last game |

**Phases** the platform drives and apps fill in:

```text
Setup ──hold──▶ Ready ──shake──▶ [Shake → Air → Settle] ──land──▶ Result ──▶ Ready …
                  │                (platform-owned)                 │
                  └──shake (trigger apps)──▶ Live ──▶ Result         └──win──▶ Over
Overlays (not app phases): Boot · Nest · Off · Menu · Adjust/Choose
```

**The trait** (static dispatch through an `enum ActiveApp`, no `dyn` or
alloc):

```rust
trait App {
    const KIND: Kind;
    fn pages(&self) -> &'static [Page];
    fn phase(&self) -> Phase;
    fn pending(&self, face: Face) -> Option<Action>; // what a hold here does
    fn hint(&self) -> Option<Hint>;                  // tap: &self, can't mutate
    fn on_land(&mut self, rng, now) -> Effects;
    fn on_shake(&mut self, now) -> Effects;          // trigger apps only
    fn commit(&mut self, a: Action, now) -> Effects; // the only mutation besides motion
    fn tick(&mut self, now) -> Effects;
    fn table(&self, now) -> TableScreen;             // T1 + optional T2 + tone
    fn held(&self, page: Page) -> HeldScreen;        // H1 value + H2 title
}
```

- **Taps can't change state.** Taps only reach `hint(&self)`.
- **One place decides what a hold does.** `resolve_hold(app, face)` returns
  the pending action, or the menu when nothing is pending (brief 2.4,
  confirmed). The face shows the action while the ring fills.
- **Screens are typed to the tiers.** `TableScreen { hero: Str<3> | Glyph,
  word: Option<Str<5> | IconRow>, tone }` can't hold a third text element or
  an over-long string. A shared number formatter shows 1000 as `1k`.
- **Choose** is the adjust mode (brief 2.2.4) offering a list of actions
  instead of a number: tip up or down, hold to confirm, shake or 25 s cancels.
- **Scoreboard** (players, tokens, scores, turn, winner) comes out of
  `pigs.rs` for every scored game.

## Menu

Hold with nothing pending opens the current app's pages, with **Apps** one
tip right (where Mode is today). Settings is an entry on Apps, including on a
die with only Dice.

- Dice: `[Apps, Count, Die]`. The dice flow is unchanged.
- Pig Toss mid-game: `[Apps, End game]`.
- Settings: one page per setting (`Brightness`, `Haptics`, `Sugar`, `Sleep`,
  `Bluetooth`, `Legal`, `Power`); tip up or down for the value, hold to save.
  Power off is a hold whose preview says `off`. **Regulatory stays on the
  die** (FCC/ISED e-labelling needs the device's own screen): the one page
  of several short H2 lines, scrolled by tipping. Nothing is engraved on the
  shell unless wanted or unavoidable. About and Owner move to the phone.

## Gestures by app

| Today | New |
|---|---|
| Pot: a tap puts the result away, then cycles bills | A tap puts the result away and shows the bills screen. A hold while a tap has it up opens Bills alone (tip ±, hold to save). A hold on a roll result, or on the label after a save, opens the menu. |
| Pig Toss: a tap banks | A hold banks, preview `bank` / `+N`. |
| Pig Toss: a tap on the win screen starts a new game | A hold opens Choose: **Rematch** (same players) or **Players** (setup). |
| Potato: a tap resets BOOM | A shake (classified `Shaking`, ignored for ~1.5 s after the boom) starts the next round; the 6 s auto-reset stays; a hold opens the menu. |
| Potato: no hold while lit | Kept. |
| Menu: a tap changes a setting or powers off | Gone with the Settings app. |
| Dice result | Total at T1 in its tone, one T2 (`3d6` or `MAX`); the parts line goes. 10d100 max shows gold `1k` with the max celebration. |

## Steps

1. **Gesture arbiter** (`gesture.rs`): grip, steady die, tap before the
   ring, cancel between ring and hold, dead until lifted. Done.
2. **App trait and platform shell** (`apps/`): Dice, Pot, Potato and Pig
   Toss behind `App`, their branches gone from the main loop, `session.rs`
   folded into `Apps`. Taps still commit through `App::tapped` until step 3.
   Rendering still reads app state directly until step 5. Done.
3. **Holds, actions, previews and tap hints**: the table above, with Choose
   built as the menu opened on one page alone (`Draft::alone`: Bills in
   hand, Next game). A hold on Pass the Pot's bills screen counts only when
   a deliberate tap brought it up, so the label after a save still lets a
   hold open the menu. Done.
4. **Apps page and Settings app**, with short names (SIM_SPEC C3 lists
   them). One `PageView` per page feeds both panels. Done.
5. **Text tiers** (SIM_SPEC X4): tier sizes per panel, generated 64×64
   cuts at 15/18/21 px (7 px is the 5×7 font, 30 px the numerals), every
   table and held screen redone to them, and the size audit
   (`tests/text_sizes.rs`). Done; a designer's hand-hinting pass over
   `tier64.rs` is still worth doing.
6. The 1 m AR check (brief test 2): by hand, not done yet.

## Open risk

Grip rejection applies in the menu too, where the die is in the hand. If a
real hand can't hold the die with one face touched, saving needs another
rule. To check on hardware.
