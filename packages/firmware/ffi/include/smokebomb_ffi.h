// C ABI of packages/firmware/ffi: the firmware core, running on the phone.
// Keep in step with src/lib.rs; the crate's tests check the constants.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Which panels the die has.
enum {
    SB_PANEL_GREY96 = 0,  // 34 mm: 96x96, 16 grey levels
    SB_PANEL_RGB64 = 1,   // 30 mm: 64x64, RGB565
};

/// Firmware ticks per second. Call sb_die_tick exactly this often per
/// simulated second: the firmware integrates the gyro over one tick.
#define SB_TICK_HZ 60
#define SB_FACE_COUNT 6

/// One IMU reading in the die's own frame (Y up, -Y is the lid).
/// At rest the face pointing up reads about +1000 mg on its axis.
typedef struct {
    int16_t accel_mg[3];
    int32_t gyro_mdps[3];
} SbImu;

typedef struct SbDie SbDie;

/// Boots a die with these panels. NULL if the firmware failed to boot.
SbDie *sb_die_new(uint8_t panel);
void sb_die_free(SbDie *die);

/// The panels' side in pixels (96 or 64).
uint32_t sb_die_panel_side(const SbDie *die);

/// Runs one tick (1/SB_TICK_HZ s) with this IMU reading and touch mask
/// (bit n = face n, in the order +X -X +Y -Y +Z -Z). Returns a counter
/// that changes whenever the panels show something new.
uint64_t sb_die_tick(SbDie *die, SbImu imu, uint8_t touch_mask);

/// Copies one face's panel as RGBA8, side*side*4 bytes, top row first, in
/// the face's own screen orientation. False if `len` is too small or the
/// face hasn't been drawn yet.
bool sb_die_face_rgba(const SbDie *die, uint8_t face, uint8_t *out, size_t len);

/// The next haptic effect the firmware played, or -1. Effects are numbered
/// in the order of smokebomb_hal::HapticEffect.
int32_t sb_die_next_haptic(SbDie *die);

/// The firmware's mode as text (for a debug readout), NUL-terminated and
/// truncated to fit. Returns the untruncated length.
size_t sb_die_mode(const SbDie *die, char *out, size_t len);

/// Sets the wall clock, seconds since local midnight.
void sb_die_set_local_time(SbDie *die, uint32_t seconds);

#ifdef __cplusplus
}
#endif
