//! C's face carousels for three of Crystal's menus: the start menu, the
//! party and the Pokédex. Like the die firmware's own menu, the selection
//! sits on the front face, its neighbours on the east (next) and west
//! (previous) faces, and a new selection slides in from the face it was on.
//!
//! The game keeps running its menu: the cube only shows it differently.
//! The selection is the game's cursor (tilting is the D-pad), A and B are
//! the game's, and whatever the game does next (open the party, a
//! submenu, save) it does as usual. Labels come from the screen or from
//! the ROM's tables, in the game's own font, at 1:1.
//!
//! * **Start menu** (over the map): the map stays on the up and back
//!   faces; each item is a card with an icon (the cube's own pixel art)
//!   and the game's label for it.
//! * **Party**: the selected Pokémon's picture on top, its card (name,
//!   level, HP, status, types) in front, its neighbours' pictures either
//!   side, and the whole party on the back. With the game's submenu open
//!   (STATS, SWITCH…), the submenu takes the front face.
//! * **Pokédex** (the list): the selected species' picture on top, its
//!   number, name and types in front, its neighbours either side, the
//!   seen and owned counts on the back.
//!
//! Pictures are decompressed from the ROM ([`crate::crystal::mons`]) and
//! kept in a small cache, so moving along the list decompresses one.

use crate::crystal::charmap;
use crate::crystal::mons::{self, text, Pic, Text};
use crate::crystal::screen::{attrmap, tilemap, Scene, COLS};
use crate::crystal::syms;
use crate::fallback::{self, find_cursor, Fallback};
use crate::geom::{Compass, Layout, Role};
use crate::mem::GbMem;
use crate::paint::{Font, Surface};
use crate::{FaceBuf, FACE};

/// Frames a new selection takes to slide onto the front face.
const SLIDE_FRAMES: i32 = 8;

const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

const HP_GREEN: u16 = rgb(48, 192, 72);
const HP_YELLOW: u16 = rgb(240, 192, 32);
const HP_RED: u16 = rgb(232, 64, 48);
const BAR_FRAME: u16 = rgb(40, 40, 48);
const BAR_EMPTY: u16 = rgb(200, 200, 208);

fn hp_colour(hp: u16, max: u16) -> u16 {
    let (hp, max) = (hp as u32, max.max(1) as u32);
    if hp * 2 > max {
        HP_GREEN
    } else if hp * 5 > max {
        HP_YELLOW
    } else {
        HP_RED
    }
}

/// The start menu: `wMenuItemsList` while the open menu is
/// `StartMenu.Items`, the cursor from `wMenuCursorY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartMenu {
    items: [u8; 9],
    n: usize,
    cursor: usize,
    /// The box's top row (0, or 2 in the Bug-Catching Contest).
    top: usize,
}

/// The party screen (`InitPartyMenuLayout`): the nicknames at (3, 1 + 2i).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Party {
    n: usize,
    /// 0..n a Pokémon, n CANCEL.
    cursor: usize,
    /// A ▶ to the right: the STATS/SWITCH/ITEM submenu is open.
    submenu: bool,
}

/// The Pokédex's list (`DEXSTATE_MAIN_SCR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dex {
    /// Index into `wPokedexOrder`.
    pos: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Start(StartMenu),
    Party(Party),
    Dex(Dex),
}

impl Screen {
    /// Which of these `scene` is, if any.
    pub fn read<M: GbMem + ?Sized>(m: &M, scene: Scene) -> Option<Screen> {
        match scene {
            Scene::OverworldUi => start_menu(m).map(Screen::Start),
            Scene::Other => party(m).map(Screen::Party).or_else(|| dex(m).map(Screen::Dex)),
            _ => None,
        }
    }

    fn kind(&self) -> u8 {
        match self {
            Screen::Start(_) => 0,
            Screen::Party(_) => 1,
            Screen::Dex(_) => 2,
        }
    }

    fn selected(&self) -> usize {
        match self {
            Screen::Start(s) => s.cursor,
            Screen::Party(p) => p.cursor,
            Screen::Dex(d) => d.pos,
        }
    }
}

fn start_menu<M: GbMem + ?Sized>(m: &M) -> Option<StartMenu> {
    let items = syms::START_MENU_ITEMS;
    if m.byte(syms::W_MENU_DATA_BANK) != items.bank
        || m.word(syms::W_MENU_DATA_POINTER_TABLE_ADDR) != items.addr
    {
        return None;
    }
    let n = m.byte(syms::W_MENU_ITEMS_LIST) as usize;
    let cursor = m.byte(syms::W_MENU_CURSOR_Y) as usize;
    if !(1..=9).contains(&n) || !(1..=n).contains(&cursor) {
        return None;
    }
    let top = [0, 2]
        .into_iter()
        .find(|&y| tilemap(m, 10, y) == charmap::FRAME_FIRST)?;
    let mut s = StartMenu {
        items: [0; 9],
        n,
        cursor: cursor - 1,
        top,
    };
    for (k, it) in s.items[..n].iter_mut().enumerate() {
        *it = m.byte(syms::W_MENU_ITEMS_LIST.offset(1 + k as u16));
    }
    Some(s)
}

fn party<M: GbMem + ?Sized>(m: &M) -> Option<Party> {
    let n = mons::party_count(m)? as usize;
    // Every nickname where the party screen prints it.
    for i in 0..n {
        let nick = mons::party_mon(m, i as u8).nickname;
        let shown = nick.as_slice().iter().take(4).enumerate();
        if nick.len == 0 || !shown.into_iter().all(|(k, &c)| tilemap(m, 3 + k, 1 + 2 * i) == c) {
            return None;
        }
    }
    let submenu = find_cursor(m, None).is_some_and(|(x, _)| x >= 10);
    let cursor = if submenu {
        m.byte(syms::W_CUR_PARTY_MON) as usize
    } else {
        (m.byte(syms::W_MENU_CURSOR_Y) as usize).wrapping_sub(1)
    };
    (cursor <= n).then_some(Party { n, cursor, submenu })
}

/// `Pokedex_DrawMainScreenBG`'s divider down column 8.
const DEX_DIVIDER: [(usize, u8); 4] = [(0, 0x59), (8, 0x53), (9, 0x54), (16, 0x5B)];

fn dex<M: GbMem + ?Sized>(m: &M) -> Option<Dex> {
    if !DEX_DIVIDER.iter().all(|&(y, t)| tilemap(m, 8, y) == t) {
        return None;
    }
    // DEXSTATE_MAIN_SCR or DEXSTATE_UPDATE_MAIN_SCR.
    if m.byte(syms::W_JUMPTABLE_INDEX) & 0x7F > 1 {
        return None;
    }
    let pos =
        m.byte(syms::W_DEX_LISTING_SCROLL_OFFSET) as usize + m.byte(syms::W_DEX_LISTING_CURSOR) as usize;
    Some(Dex { pos })
}

fn dex_species<M: GbMem + ?Sized>(m: &M, pos: usize) -> u8 {
    if pos > 0xFF {
        return 0;
    }
    m.byte(syms::W_POKEDEX_ORDER.offset(pos as u16))
}

/// A few decompressed pictures, by species, letter and shininess.
struct PicCache {
    slots: [(u32, Option<Pic>); 4],
    next: usize,
}

impl PicCache {
    fn get<M: GbMem + ?Sized>(&mut self, m: &M, species: u8, letter: u8) -> Option<&Pic> {
        let key = 1 << 16 | (letter as u32) << 8 | species as u32;
        let i = match self.slots.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let i = self.next;
                self.next = (self.next + 1) % self.slots.len();
                self.slots[i] = (key, mons::front_pic(m, species, letter));
                i
            }
        };
        self.slots[i].1.as_ref()
    }
}

/// The carousels' state: the last selection, the slide, the pictures.
pub struct Menus {
    last: Option<(u8, usize)>,
    /// The selection sliding off, and the direction the new one comes from
    /// (+1 from the east), and frames in.
    slide: Option<(usize, i32, i32)>,
    pics: PicCache,
}

impl Default for Menus {
    fn default() -> Self {
        Self::new()
    }
}

impl Menus {
    pub const fn new() -> Self {
        const NONE: (u32, Option<Pic>) = (0, None);
        Menus {
            last: None,
            slide: None,
            pics: PicCache {
                slots: [NONE; 4],
                next: 0,
            },
        }
    }

    /// Forget the selection (another screen is showing).
    pub fn idle(&mut self) {
        self.last = None;
        self.slide = None;
    }

    /// Draw `screen`. For the start menu the up, back and bottom faces are
    /// left as they are (the map).
    pub fn draw<M: GbMem + ?Sized>(
        &mut self,
        m: &M,
        screen: &Screen,
        layout: &Layout,
        faces: &mut [FaceBuf; 6],
        fb: &Fallback,
    ) {
        let sel = screen.selected();
        let len = match screen {
            Screen::Start(s) => s.n,
            Screen::Party(p) => p.n + 1,
            Screen::Dex(_) => 0x100,
        };
        // A new selection: slide it in from the side it was on.
        match self.last {
            Some((k, old)) if k == screen.kind() && old != sel => {
                let dir = if sel == (old + 1) % len {
                    1
                } else if (sel + 1) % len == old {
                    -1
                } else if sel > old {
                    1
                } else {
                    -1
                };
                self.slide = Some((old, dir, 0));
            }
            Some((k, _)) if k == screen.kind() => {}
            _ => self.slide = None,
        }
        self.last = Some((screen.kind(), sel));

        let neighbour = |d: isize| -> Option<usize> {
            let i = sel as isize + d;
            match screen {
                Screen::Start(_) => Some(i.rem_euclid(len as isize) as usize),
                _ => (0..len as isize).contains(&i).then_some(i as usize),
            }
        };
        let font = Font::new(m, self.text_attr(m, screen));
        let paper = font.paper();

        // Front: the selection, sliding in over the old one.
        let front = Role::Side(Compass::South);
        let submenu = matches!(screen, Screen::Party(p) if p.submenu)
            && fallback::front_panel(m, None, layout, faces, fb);
        if submenu {
            self.slide = None;
        } else {
            match self.slide {
                Some((old, dir, t)) if t < SLIDE_FRAMES => {
                    let p = (t + 1) * FACE as i32 / SLIDE_FRAMES;
                    self.card(
                        m,
                        screen,
                        &font,
                        old,
                        &mut Surface::new(faces, layout, front, -dir * p),
                    );
                    self.card(
                        m,
                        screen,
                        &font,
                        sel,
                        &mut Surface::new(faces, layout, front, dir * (FACE as i32 - p)),
                    );
                    self.slide = Some((old, dir, t + 1));
                }
                _ => {
                    self.slide = None;
                    self.card(m, screen, &font, sel, &mut Surface::new(faces, layout, front, 0));
                }
            }
        }

        // Either side: the neighbours, as pictures (or icons).
        for (c, d) in [(Compass::East, 1), (Compass::West, -1)] {
            let mut s = Surface::new(faces, layout, Role::Side(c), 0);
            match neighbour(d) {
                Some(i) => self.side_card(m, screen, &font, i, &mut s),
                None => s.fill(paper),
            }
        }

        // Top and back.
        match screen {
            Screen::Start(_) => {}
            Screen::Party(p) => {
                self.picture(
                    m,
                    screen,
                    &font,
                    sel,
                    &mut Surface::new(faces, layout, Role::Top, 0),
                );
                party_overview(
                    m,
                    p,
                    &font,
                    &mut Surface::new(faces, layout, Role::Side(Compass::North), 0),
                );
            }
            Screen::Dex(_) => {
                self.picture(
                    m,
                    screen,
                    &font,
                    sel,
                    &mut Surface::new(faces, layout, Role::Top, 0),
                );
                dex_counts(
                    m,
                    &font,
                    &mut Surface::new(faces, layout, Role::Side(Compass::North), 0),
                );
            }
        }
    }

    /// The BG palette the screen's text is in.
    fn text_attr<M: GbMem + ?Sized>(&self, m: &M, screen: &Screen) -> u8 {
        match screen {
            Screen::Start(s) => attrmap(m, 12, s.top + 2),
            Screen::Party(_) => attrmap(m, 3, 1),
            Screen::Dex(_) => attrmap(m, 1, 11),
        }
    }

    /// The front card for entry `i`.
    fn card<M: GbMem + ?Sized>(&mut self, m: &M, screen: &Screen, font: &Font<M>, i: usize, s: &mut Surface) {
        s.fill(font.paper());
        match screen {
            Screen::Start(st) => start_card(m, st, font, i, s),
            Screen::Party(p) if i >= p.n => font.centred_text(s, &text("CANCEL"), 28),
            Screen::Party(_) => mon_card(m, font, &mons::party_mon(m, i as u8), s),
            Screen::Dex(_) => dex_card(m, font, dex_species(m, i), s),
        }
    }

    /// A neighbour: its picture and name (the start menu: the item card).
    fn side_card<M: GbMem + ?Sized>(
        &mut self,
        m: &M,
        screen: &Screen,
        font: &Font<M>,
        i: usize,
        s: &mut Surface,
    ) {
        s.fill(font.paper());
        match screen {
            Screen::Start(st) => start_card(m, st, font, i, s),
            Screen::Party(p) if i >= p.n => font.centred_text(s, &text("CANCEL"), 28),
            Screen::Party(_) => {
                let mon = mons::party_mon(m, i as u8);
                if !mon.egg {
                    let pal = mons::palette(m, mon.species, mon.shiny());
                    if let Some(pic) = self.pics.get(m, mon.species, mon.letter()) {
                        s.pic(pic, &pal, 4, 0);
                    }
                }
                font.centred_text(s, &mon.nickname, 56);
            }
            Screen::Dex(_) => {
                let sp = dex_species(m, i);
                if mons::seen(m, sp) {
                    let pal = mons::palette(m, sp, false);
                    if let Some(pic) = self.pics.get(m, sp, 1) {
                        s.pic(pic, &pal, 4, 0);
                    }
                    font.centred_text(s, &mons::species_name(m, sp), 56);
                } else if mons::is_species(sp) {
                    font.big(s, mons::code(b'?'), 32, 26, 3);
                    font.centred_text(s, &text("-----"), 56);
                }
            }
        }
    }

    /// The up face: entry `i`'s picture.
    fn picture<M: GbMem + ?Sized>(
        &mut self,
        m: &M,
        screen: &Screen,
        font: &Font<M>,
        i: usize,
        s: &mut Surface,
    ) {
        s.fill(font.paper());
        let (species, letter, shiny) = match screen {
            Screen::Party(p) if i < p.n => {
                let mon = mons::party_mon(m, i as u8);
                if mon.egg {
                    font.centred_text(s, &mon.nickname, 28);
                    return;
                }
                (mon.species, mon.letter(), mon.shiny())
            }
            Screen::Dex(_) => {
                let sp = dex_species(m, i);
                if !mons::seen(m, sp) {
                    if mons::is_species(sp) {
                        font.big(s, mons::code(b'?'), 32, 32, 4);
                    }
                    return;
                }
                (sp, 1, false)
            }
            _ => return,
        };
        let pal = mons::palette(m, species, shiny);
        if let Some(pic) = self.pics.get(m, species, letter) {
            s.pic(pic, &pal, 4, 4);
        }
    }
}

/// A start menu item: its icon, and its label as the menu prints it.
fn start_card<M: GbMem + ?Sized>(m: &M, st: &StartMenu, font: &Font<M>, i: usize, s: &mut Surface) {
    let (rows, pal) = &ICONS[(st.items[i] as usize).min(ICONS.len() - 1)];
    s.icon(rows, pal, 16, 6, 2);
    // The label: from the cursor slot's right, to the box's edge.
    let y = st.top + 2 + 2 * i;
    let mut label = [charmap::SPACE; 8];
    let mut n = 0;
    for x in 12..COLS - 1 {
        label[n] = tilemap(m, x, y);
        n += 1;
    }
    while n > 0 && label[n - 1] == charmap::SPACE {
        n -= 1;
    }
    font.centred(s, &label[..n], 46, false);
}

/// The selected party member: name, level and status, HP, types.
fn mon_card<M: GbMem + ?Sized>(m: &M, font: &Font<M>, mon: &mons::PartyMon, s: &mut Surface) {
    font.centred_text(s, &mon.nickname, 3);
    if mon.egg {
        return;
    }
    let lv = text("Lv").then(Text::number(mon.level as u16, 3).as_slice());
    font.text(s, lv.as_slice(), 4, 14, 40, false);
    if let Some(st) = mons::status_name(mon.status) {
        font.text(s, text(st).as_slice(), 40, 14, 24, false);
    }
    let colours = [hp_colour(mon.hp, mon.max_hp), BAR_FRAME, BAR_EMPTY];
    s.bar(6, 26, 52, (mon.hp, mon.max_hp), colours);
    let hp = Text::number(mon.hp, 3)
        .then(&[mons::code(b'/')])
        .then(Text::number(mon.max_hp, 3).as_slice());
    font.centred_text(s, &hp, 34);
    types(m, font, mon.species, s, 46);
}

fn types<M: GbMem + ?Sized>(m: &M, font: &Font<M>, species: u8, s: &mut Surface, v: i32) {
    let (a, b) = mons::types(m, species);
    font.centred_text(s, &mons::type_name(m, a), v);
    if b != a {
        font.centred_text(s, &mons::type_name(m, b), v + 9);
    }
}

/// A Pokédex entry: number, name, owned or seen, types.
fn dex_card<M: GbMem + ?Sized>(m: &M, font: &Font<M>, species: u8, s: &mut Surface) {
    if !mons::is_species(species) {
        return;
    }
    let no = text("No.").then(Text::number(species as u16, 3).as_slice());
    font.centred_text(s, &no, 3);
    if !mons::seen(m, species) {
        font.centred_text(s, &text("-----"), 16);
        return;
    }
    font.centred_text(s, &mons::species_name(m, species), 16);
    let own = if mons::caught(m, species) { "OWN" } else { "SEEN" };
    font.centred_text(s, &text(own), 28);
    types(m, font, species, s, 44);
}

/// The back face in the party: everyone, the selection inverted, an HP
/// pip each.
fn party_overview<M: GbMem + ?Sized>(m: &M, p: &Party, font: &Font<M>, s: &mut Surface) {
    s.fill(font.paper());
    for i in 0..=p.n {
        let v = 1 + 9 * i as i32;
        let on = i == p.cursor;
        if i == p.n {
            font.text(s, text("CANCEL").as_slice(), 4, v, 48, on);
            continue;
        }
        let mon = mons::party_mon(m, i as u8);
        font.text(s, mon.nickname.as_slice(), 4, v, 48, on);
        if !mon.egg {
            s.rect(55, v + 2, 5, 5, BAR_FRAME);
            s.rect(56, v + 3, 3, 3, hp_colour(mon.hp, mon.max_hp));
        }
    }
}

/// The back face in the Pokédex.
fn dex_counts<M: GbMem + ?Sized>(m: &M, font: &Font<M>, s: &mut Surface) {
    s.fill(font.paper());
    font.centred_text(s, &text("SEEN"), 8);
    font.centred_text(s, &Text::number(mons::seen_count(m), 3), 18);
    font.centred_text(s, &text("OWN"), 36);
    font.centred_text(s, &Text::number(mons::caught_count(m), 3), 46);
}

/// The start menu's icons, by `STARTMENUITEM_*`: the cube's own pixel art.
type Icon = ([&'static str; 16], [u16; 3]);
const ICONS: [Icon; 9] = [
    // POKéDEX: a red handheld.
    (
        [
            "",
            "  333333333333",
            "  311111111113",
            "  313333333313",
            "  312222222213",
            "  312222222213",
            "  312222222213",
            "  312222222213",
            "  313333333313",
            "  311111111113",
            "  311331133113",
            "  311331133113",
            "  311111111113",
            "  311111111113",
            "  333333333333",
            "",
        ],
        [rgb(224, 56, 56), rgb(152, 216, 232), rgb(56, 32, 40)],
    ),
    // POKéMON: a ball.
    (
        [
            "     333333",
            "   3311111133",
            "  311111111113",
            " 31111111111113",
            " 31111111111113",
            "3111111111111113",
            "3111113333111113",
            "3333332222333333",
            "3333332222333333",
            "3222223333222223",
            "3222222222222223",
            " 32222222222223",
            " 32222222222223",
            "  322222222223",
            "   3322222233",
            "     333333",
        ],
        [rgb(232, 48, 48), rgb(248, 248, 248), rgb(32, 32, 40)],
    ),
    // PACK: a bag.
    (
        [
            "",
            "     333333",
            "    3      3",
            "    3      3",
            "  333333333333",
            " 31111111111113",
            " 31111111111113",
            " 32222222222223",
            " 31111222211113",
            " 31111211211113",
            " 31111222211113",
            " 31111111111113",
            " 31111111111113",
            " 31111111111113",
            "  333333333333",
            "",
        ],
        [rgb(216, 168, 72), rgb(144, 96, 40), rgb(64, 40, 24)],
    ),
    // Your name: a trainer card.
    (
        [
            "",
            "",
            " 33333333333333",
            " 31111111111113",
            " 31222111111113",
            " 32222213333313",
            " 32222211111113",
            " 31222113333313",
            " 32222211111113",
            " 32222213333313",
            " 31111111111113",
            " 31333333333313",
            " 31111111111113",
            " 33333333333333",
            "",
            "",
        ],
        [rgb(240, 232, 200), rgb(80, 128, 224), rgb(48, 48, 64)],
    ),
    // SAVE: a report.
    (
        [
            "",
            "  333333333333",
            "  311111111113",
            "  311111111113",
            "  311222222113",
            "  311333333113",
            "  311222222113",
            "  311111111113",
            "  311111111113",
            "  311111111113",
            "  311111111113",
            "  311111111113",
            "  322222222223",
            "  333333333333",
            "",
            "",
        ],
        [rgb(64, 120, 216), rgb(248, 248, 240), rgb(32, 40, 72)],
    ),
    // OPTION: sliders.
    (
        [
            "",
            "    333",
            "2222313222222222",
            "    333",
            "",
            "          333",
            "2222222222313222",
            "          333",
            "",
            "      333",
            "2222223132222222",
            "      333",
            "",
            "",
            "",
            "",
        ],
        [rgb(248, 248, 248), rgb(136, 136, 152), rgb(48, 48, 64)],
    ),
    // EXIT: out of the door.
    (
        [
            "",
            " 3333333",
            " 3111113",
            " 3111113",
            " 3111113    3",
            " 3111113    33",
            " 3111113 222223",
            " 3113113 2222223",
            " 3111113 222223",
            " 3111113    33",
            " 3111113    3",
            " 3111113",
            " 3111113",
            " 3333333",
            "",
            "",
        ],
        [rgb(184, 120, 64), rgb(56, 176, 88), rgb(48, 32, 24)],
    ),
    // POKéGEAR.
    (
        [
            "",
            "   3333333333",
            "   3111111113",
            "   3133333313",
            "   3132222313",
            "   3132222313",
            "   3132222313",
            "   3133333313",
            "   3111111113",
            "   3131313113",
            "   3111111113",
            "   3131313113",
            "   3111111113",
            "   3131313113",
            "   3333333333",
            "",
        ],
        [rgb(96, 104, 120), rgb(168, 224, 160), rgb(32, 32, 40)],
    ),
    // QUIT (the Bug-Catching Contest): a flag.
    (
        [
            "",
            "  3",
            "  33333333",
            "  31111111333",
            "  31111111113",
            "  31111111113",
            "  31111111113",
            "  33333331113",
            "  2      3333",
            "  2",
            "  2",
            "  2",
            "  2",
            "  2",
            " 333",
            "",
        ],
        [rgb(232, 64, 48), rgb(136, 136, 152), rgb(40, 40, 48)],
    ),
];
