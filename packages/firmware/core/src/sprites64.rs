//! Pixel sprites for the 64×64 screens, hand-placed on the panel grid.
//!
//! One-colour sprites are [`Glyph`]s drawn in whatever colour the screen
//! picks. The sugar-cube logo has three tones, kept as one picture with a
//! letter per tone and split into layers ([`layer`]).

use crate::font64::{glyph, layer, Glyph};

// ---------- the dice (19×19 line art, 1 px lines) ----------
//
// Each is the die's solid as the 96×96 setup label draws it (icons.rs):
// d4 a tetrahedron, d6 a cube, d8 an octahedron, d10 a kite, d12 a
// dodecahedron, d20 an icosahedron. Lines run at whole-pixel slopes (1:1,
// 2:1) where the solid allows, and every icon is mirror-symmetric.

pub static D4: Glyph<19> = glyph(
    ' ',
    concat!(
        "...................\n",
        ".........#.........\n",
        "........###........\n",
        "........###........\n",
        ".......#.#.#.......\n",
        ".......#.#.#.......\n",
        "......#..#..#......\n",
        "......#..#..#......\n",
        ".....#...#...#.....\n",
        ".....#...#...#.....\n",
        "....#....#....#....\n",
        "....#....#....#....\n",
        "...#...##.##...#...\n",
        "...#..#.....#..#...\n",
        "..#.##.......##.#..\n",
        "..##...........##..\n",
        ".#################.\n",
        "...................\n",
        "...................\n",
    ),
);

pub static D6: Glyph<19> = glyph(
    ' ',
    concat!(
        "...................\n",
        ".........#.........\n",
        ".......##.##.......\n",
        ".....##.....##.....\n",
        "...##.........##...\n",
        ".##.............##.\n",
        ".#.##.........##.#.\n",
        ".#...##.....##...#.\n",
        ".#.....##.##.....#.\n",
        ".#.......#.......#.\n",
        ".#.......#.......#.\n",
        ".#.......#.......#.\n",
        ".#.......#.......#.\n",
        ".##......#......##.\n",
        "...##....#....##...\n",
        ".....##..#..##.....\n",
        ".......#####.......\n",
        ".........#.........\n",
        "...................\n",
    ),
);

pub static D8: Glyph<19> = glyph(
    ' ',
    concat!(
        ".........#.........\n",
        "........#.#........\n",
        ".......#...#.......\n",
        "......#.....#......\n",
        ".....#.......#.....\n",
        ".....#.......#.....\n",
        "....#.........#....\n",
        "...#...........#...\n",
        "..#.............#..\n",
        ".#################.\n",
        "..#.............#..\n",
        "...#...........#...\n",
        "....#.........#....\n",
        ".....#.......#.....\n",
        ".....#.......#.....\n",
        "......#.....#......\n",
        ".......#...#.......\n",
        "........#.#........\n",
        ".........#.........\n",
    ),
);

pub static D10: Glyph<19> = glyph(
    ' ',
    concat!(
        ".........#.........\n",
        "........#.#........\n",
        ".......#...#.......\n",
        "......#.....#......\n",
        ".....#.......#.....\n",
        "....#.........#....\n",
        "...#...........#...\n",
        "..#.............#..\n",
        ".##.............##.\n",
        "..###.........###..\n",
        "...#.##.....##.#...\n",
        "...#...##.##...#...\n",
        "....#....#....#....\n",
        ".....#...#...#.....\n",
        "......#..#..#......\n",
        ".......#.#.#.......\n",
        ".......#.#.#.......\n",
        "........###........\n",
        ".........#.........\n",
    ),
);

pub static D12: Glyph<19> = glyph(
    ' ',
    concat!(
        ".........#.........\n",
        "........###........\n",
        "......##.#.##......\n",
        ".....#...#...#.....\n",
        "....#....#....#....\n",
        "..##.....#.....##..\n",
        ".#.....##.##.....#.\n",
        "###...#.....#...###\n",
        "#..###.......###..#\n",
        ".#...#.......#...#.\n",
        ".#....#.....#....#.\n",
        "..#...#.....#...#..\n",
        "..#....#...#....#..\n",
        "..#....#####....#..\n",
        "...#..#.....#..#...\n",
        "...#..#.....#..#...\n",
        "....##.......##....\n",
        "....###########....\n",
        "...................\n",
    ),
);

pub static D20: Glyph<19> = glyph(
    ' ',
    concat!(
        ".........#.........\n",
        ".......#####.......\n",
        ".....##..#..##.....\n",
        "...##....#....##...\n",
        ".##......#......##.\n",
        ".#......#.#......#.\n",
        ".#.....#...#.....#.\n",
        ".#.....#...#.....#.\n",
        ".#....#.....#....#.\n",
        ".#...#.......#...#.\n",
        ".#...#.......#...#.\n",
        ".#..#.........#..#.\n",
        ".#.#...........#.#.\n",
        ".#.#############.#.\n",
        ".##.#.........#.##.\n",
        "...####.....####...\n",
        ".....###...###.....\n",
        ".......##.##.......\n",
        ".........#.........\n",
    ),
);

// ---------- the sugar cube (boot, 15×15) ----------

/// T top face, L left face, R right face: an isometric cube in 2:1 steps.
const CUBE: &str = "
.......T.......
.....TTTTT.....
...TTTTTTTTT...
.TTTTTTTTTTTTT.
LLTTTTTTTTTTTRR
LLLLTTTTTTTRRRR
LLLLLLTTTRRRRRR
LLLLLLLRRRRRRRR
LLLLLLLRRRRRRRR
LLLLLLLRRRRRRRR
LLLLLLLRRRRRRRR
.LLLLLLRRRRRRR.
...LLLLRRRRR...
.....LLRRR.....
.......R.......
";
pub static CUBE_TOP: Glyph<15> = layer(' ', CUBE, b'T');
pub static CUBE_LEFT: Glyph<15> = layer(' ', CUBE, b'L');
pub static CUBE_RIGHT: Glyph<15> = layer(' ', CUBE, b'R');

/// A four-point sparkle, 5×5.
pub static SPARKLE: Glyph<5> = glyph(' ', "..#.. ..#.. ##.## ..#.. ..#..");

// ---------- small things ----------

/// A pip, 10×10: the same size against its 14 px grid as the 96×96 die's
/// pips against theirs (radius ≈ a third of the spacing).
pub static PIP: Glyph<10> = glyph(
    ' ',
    "
    ...####...
    .########.
    .########.
    ##########
    ##########
    ##########
    ##########
    .########.
    .########.
    ...####...
    ",
);

/// The menu's arrows, 7×4.
pub static UP: Glyph<4> = glyph(' ', "...#... ..###.. .#####. #######");
pub static DOWN: Glyph<4> = glyph(' ', "####### .#####. ..###.. ...#...");

/// Saved: a check, 11×8.
pub static CHECK: Glyph<8> = glyph(
    ' ',
    "
    ..........#
    .........##
    ........##.
    #......##..
    ##....##...
    .##..##....
    ..####.....
    ...##......
    ",
);

/// The charge bolt, 9×14.
pub static BOLT: Glyph<14> = glyph(
    ' ',
    "
    ....#####
    ...#####.
    ...####..
    ..####...
    .####....
    .########
    ########.
    ....###..
    ...###...
    ...##....
    ..##.....
    ..#......
    .#.......
    #........
    ",
);

/// The status bar's battery, 11×5 outline; the level fills its inside.
pub static BATTERY: Glyph<5> = glyph(' ', "#########.. #.......### #.......### #.......### #########..");

// ---------- Pig Toss ----------

/// The players' symbols (the 96×96 die draws them as shapes; these are the
/// same six at 64×64, 9 wide and 7 tall like a capital): top hat, car,
/// boot, boat, crown, star.
pub static HAT: Glyph<7> = glyph(
    ' ',
    "..#####.. ..#####.. ..#####.. ..#####.. ..#...#.. ..#####.. #########",
);
pub static CAR: Glyph<7> = glyph(
    ' ',
    "......... ..####... .#..#.#.. ######### ######### .##...##. .##...##.",
);
pub static BOOT: Glyph<7> = glyph(
    ' ',
    "..###.... ..###.... ..###.... ..####... ..######. ..####### ..#######",
);
pub static BOAT: Glyph<7> = glyph(
    ' ',
    "....#.... ....##... ....###.. ....#.... ######### .#######. ..#####..",
);
pub static CROWN: Glyph<7> = glyph(
    ' ',
    "#...#...# ##.###.## ######### ######### ######### ......... #########",
);
pub static STAR: Glyph<7> = glyph(
    ' ',
    "....#.... ...###... ######### .#######. ..#####.. .###.###. .#.....#.",
);

/// A padlock: the shackle (lifted while it's open) over the body, with
/// the keyhole cut out. 13 wide.
pub static SHACKLE: Glyph<6> = glyph(
    ' ',
    "...#######... ..##.....##.. ..#.......#.. ..#.......#.. ..#.......#.. ..#.......#..",
);
pub static LOCK_BODY: Glyph<9> = glyph(
    ' ',
    "############# ############# ############# ######.###### #####...##### ######.###### ######.###### ############# #############",
);

/// A happy pig's face, 19×16: `#` face, `o` snout, `x` eyes, nostrils and
/// smile.
const PIG: &str = "
.##.............##.
.###...........###.
.####.#######.####.
..###############..
.#################.
####xx#######xx####
####xx#######xx####
###################
######ooooooo######
#####ooxoooxoo#####
######ooooooo######
.#################.
.####x#######x####.
..####xxxxxxx####..
...#############...
.....#########.....
";
pub static PIG_FACE: Glyph<16> = layer(' ', PIG, b'#');
pub static PIG_SNOUT: Glyph<16> = layer(' ', PIG, b'o');
pub static PIG_DARK: Glyph<16> = layer(' ', PIG, b'x');

/// A smooch, 9×8.
pub static HEART: Glyph<8> = glyph(
    ' ',
    ".##...##. ####.#### ######### ######### .#######. ..#####.. ...###... ....#....",
);

/// Shake: a die with motion lines, 9×5.
pub static SHAKE: Glyph<5> = glyph(' ', "....##### .#..#...# ....#.#.# ##..#...# ....#####");

/// Tap, 7×7.
pub static TAP: Glyph<7> = glyph(' ', "..###.. .#...#. #..#..# #.###.# #..#..# .#...#. ..###..");

/// The hot potato, 16×13: `#` skin, `s` its spots.
const POTATO: &str = "
....########....
..############..
.###s##########.
################
################
################
##########s#####
################
################
.###s##########.
.##############.
..############..
....########....
";
pub static POTATO_SKIN: Glyph<13> = layer(' ', POTATO, b'#');
pub static POTATO_SPOTS: Glyph<13> = layer(' ', POTATO, b's');
/// Where the fuse leaves the potato, pixels from its top left.
pub const POTATO_FUSE: (f32, f32) = (9.0, 0.0);

/// The potato's face as the fuse heats, drawn over the skin (same 16×13
/// frame): calm, worried, panicking, and burnt out.
pub static FACE_CALM: Glyph<13> = glyph(
    ' ',
    "................ ................ ................ ................ \
     .....#....#..... .....#....#..... ................ ......#..#...... \
     .......##.......",
);
pub static FACE_WORRIED: Glyph<13> = glyph(
    ' ',
    "................ ................ ................ ....##....##.... \
     .....#....#..... .....#....#..... ................ ................ \
     ......####......",
);
pub static FACE_PANIC: Glyph<13> = glyph(
    ' ',
    "................ ................ ...##......##... ................ \
     ....##....##.... ....##....##.... ................ .......##....... \
     ......#..#...... ......#..#...... .......##.......",
);
pub static FACE_BURNT: Glyph<13> = glyph(
    ' ',
    "................ ................ ................ ....#.#..#.#.... \
     .....#....#..... ....#.#..#.#.... ................ ................ \
     ......####......",
);

/// A drop of sweat, 3×4.
pub static SWEAT: Glyph<4> = glyph(' ', ".#. .#. ### .#.");

/// A spark, 5×5, in two shapes to flicker between.
pub static SPARK_X: Glyph<5> = glyph(' ', "#...# .#.#. ..#.. .#.#. #...#");
pub static SPARK_PLUS: Glyph<5> = glyph(' ', "..#.. ..#.. ##.## ..#.. ..#..");

/// A banknote, 17×9: a border, a dot in each corner and the middle's oval.
pub static BILL: Glyph<9> = glyph(
    ' ',
    "################# #...............# #.#....###....#.# #.....#...#.....# \
     #.....#.#.#.....# #.....#...#.....# #.#....###....#.# #...............# \
     #################",
);

/// Pass the Pot's faces, 13 wide: pass a bill left or right, feed the pot
/// (`#` the cauldron, `o` its inside), or keep it.
pub static PASS_RIGHT: Glyph<9> = glyph(
    ' ',
    ".......#..... .......##.... .......###... ############. ############# \
     ############. .......###... .......##.... .......#.....",
);
pub static PASS_LEFT: Glyph<9> = glyph(
    ' ',
    ".....#....... ....##....... ...###....... .############ ############# \
     .############ ...###....... ....##....... .....#.......",
);
const POT: &str = "
#############
.#ooooooooo#.
##ooooooooo##
##ooooooooo##
.#ooooooooo#.
..#ooooooo#..
...#######...
..##.....##..
";
pub static POT_RIM: Glyph<8> = layer(' ', POT, b'#');
pub static POT_INSIDE: Glyph<8> = layer(' ', POT, b'o');
pub static KEEP: Glyph<5> = glyph(' ', ".###. ##### ##### ##### .###.");

/// Settings: a gear, 15×15, eight teeth round a hole.
pub static GEAR: Glyph<15> = glyph(
    ' ',
    "......###...... ......###...... ..##..###..##.. ..###########.. \
     ...#########... ...###...###... #####.....##### #####.....##### \
     #####.....##### ...###...###... ...#########... ..###########.. \
     ..##..###..##.. ......###...... ......###......",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn symmetric<const H: usize>(g: &Glyph<H>) -> bool {
        let w = g.width as u32;
        g.rows.iter().all(|&r| {
            let bits = r >> (32 - w);
            bits.reverse_bits() >> (32 - w) == bits
        })
    }

    #[test]
    fn dice_are_19_square_and_symmetric() {
        for g in [&D4, &D6, &D8, &D10, &D12, &D20] {
            assert_eq!(g.width, 19);
            assert!(symmetric(g));
        }
    }

    #[test]
    fn the_cube_layers_tile_without_overlap() {
        for y in 0..15 {
            let (t, l, r) = (CUBE_TOP.rows[y], CUBE_LEFT.rows[y], CUBE_RIGHT.rows[y]);
            assert_eq!(t & l, 0);
            assert_eq!(t & r, 0);
            assert_eq!(l & r, 0);
        }
        assert!(symmetric(&PIP) && symmetric(&UP) && symmetric(&DOWN) && symmetric(&SPARKLE));
        for g in [&PIG_FACE, &PIG_SNOUT, &PIG_DARK] {
            assert!(symmetric(g));
        }
        assert!(symmetric(&HEART) && symmetric(&TAP) && symmetric(&SHACKLE) && symmetric(&LOCK_BODY));
        assert!(symmetric(&BILL) && symmetric(&POT_RIM) && symmetric(&POT_INSIDE) && symmetric(&KEEP));
        assert!(symmetric(&SPARK_X) && symmetric(&SPARK_PLUS) && symmetric(&SWEAT));
        for face in [&FACE_CALM, &FACE_WORRIED, &FACE_PANIC, &FACE_BURNT] {
            assert!(symmetric(face));
            // The face sits on the skin.
            for y in 0..13 {
                assert_eq!(face.rows[y] & !POTATO_SKIN.rows[y] & !POTATO_SPOTS.rows[y], 0);
            }
        }
        // The arrows mirror each other.
        for y in 0..9 {
            let w = PASS_LEFT.width as u32;
            let l = PASS_LEFT.rows[y] >> (32 - w);
            assert_eq!(l.reverse_bits() >> (32 - w), PASS_RIGHT.rows[y] >> (32 - w));
        }
        for y in 0..16 {
            assert_eq!(PIG_FACE.rows[y] & (PIG_SNOUT.rows[y] | PIG_DARK.rows[y]), 0);
        }
    }
}
