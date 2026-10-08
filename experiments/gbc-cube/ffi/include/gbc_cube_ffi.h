// C ABI of experiments/gbc-cube/ffi: a Game Boy Color game on the die, for
// the iOS app's Simulator tab. Keep in step with src/lib.rs.
//
// The same library also exports the die firmware's ABI (smokebomb_ffi.h), so
// the app links this one library instead of libsmokebomb_ffi.a.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// One IMU reading in the die's frame: the same layout as smokebomb_ffi.h's
/// SbImu (milli-g, milli-degrees per second).
typedef struct {
    int16_t accel_mg[3];
    int32_t gyro_mdps[3];
} GcImu;

/// Joypad bits for gc_cube_tick's `keys`.
enum {
    GC_KEY_A = 0x01,
    GC_KEY_B = 0x02,
    GC_KEY_SELECT = 0x04,
    GC_KEY_START = 0x08,
    GC_KEY_RIGHT = 0x10,
    GC_KEY_LEFT = 0x20,
    GC_KEY_UP = 0x40,
    GC_KEY_DOWN = 0x80,
};

/// How text, menus and battles are laid out (the experiment's Phase 4).
enum {
    GC_UI_FRONT = 0,   // C: on the front face, the world kept (default)
    GC_UI_PAN = 1,     // A: the frame panned to what's active
    GC_UI_SPREAD = 2,  // B: the frame spread over fixed faces
};

typedef struct GcCube GcCube;

/// Boots the ROM at `rom_path` (a .gb/.gbc file). Its battery save is read
/// from, and written to, the same path with a .sav extension. NULL
/// `rom_path` boots the built-in demo cart. Returns NULL if it can't boot;
/// gc_cube_last_error says why.
GcCube *gc_cube_open(const char *rom_path);

/// Why the last gc_cube_open failed, NUL-terminated and truncated to fit.
/// Returns the untruncated length (0 if there's no error).
size_t gc_cube_last_error(char *out, size_t len);

/// Writes the save if it changed, then frees the cube.
void gc_cube_free(GcCube *cube);

/// The panels' side in pixels (64).
uint32_t gc_cube_panel_side(const GcCube *cube);

/// Runs one 1/60 s tick: the cube senses `imu` and `touch_mask` (bit n =
/// face n, +X -X +Y -Y +Z -Z), `keys` are pressed on top, the emulator runs
/// a frame and the faces are drawn. Returns a counter that changes whenever
/// the faces do.
uint64_t gc_cube_tick(GcCube *cube, GcImu imu, uint8_t touch_mask, uint8_t keys);

/// One face as RGBA8, 64*64*4 bytes, top row first, in the face's own
/// screen orientation. False if `len` is too small.
bool gc_cube_face_rgba(const GcCube *cube, uint8_t face, uint8_t *out, size_t len);

/// A one-line status ("Overworld, world from RAM, up +Y"), as gc_cube_last_error.
size_t gc_cube_status(const GcCube *cube, char *out, size_t len);

/// The cartridge's title from its header, as gc_cube_last_error.
size_t gc_cube_title(const GcCube *cube, char *out, size_t len);

/// Text, menus and battles: GC_UI_FRONT, GC_UI_PAN or GC_UI_SPREAD.
void gc_cube_set_ui(GcCube *cube, uint8_t style);

/// The overworld from game RAM (true, the default) or the emulator's frame
/// folded over the cube (false).
void gc_cube_set_world(GcCube *cube, bool from_ram);

#ifdef __cplusplus
}
#endif
