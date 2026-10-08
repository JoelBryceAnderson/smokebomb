/* The C ABI the Rust crate binds to (see shim.c). Keep in step with
 * emu/src/ffi.rs. */
#ifndef GCB_SHIM_H
#define GCB_SHIM_H

#include <stddef.h>
#include <stdint.h>

#define GCB_LCD_WIDTH 160
#define GCB_LCD_HEIGHT 144

struct gcb;

struct gcb_mem {
	uint8_t *wram;
	size_t wram_len;
	uint8_t *vram;
	size_t vram_len;
	uint8_t *oam;
	/* 0xFF00-0xFFFF: I/O registers then HRAM, indexed by addr - 0xFF00. */
	uint8_t *hram_io;
	/* CGB palette RAM, 64 bytes each, BGR555 little endian. */
	uint8_t *bg_palette;
	uint8_t *obj_palette;
	/* The same 64 colours converted to RGB565 (BG 0-31, OBJ 32-63). */
	uint16_t *fix_palette;
};

struct gcb_state {
	uint8_t cgb_mode;
	uint8_t double_speed;
	uint8_t halted;
	uint8_t wram_bank;
	uint8_t vram_bank;
	uint16_t rom_bank;
	uint16_t pc;
	uint16_t sp;
};

struct gcb_stats {
	uint64_t rom_reads;
	/* Reads whose 16 KiB bank differs from the previous read's. */
	uint64_t bank_changes;
	uint64_t cart_ram_writes;
	uint64_t frames;
	/* Bitmap of the 16 KiB banks read since the last take (512 banks). */
	uint8_t banks_touched[64];
	/* Reads per 16 KiB bank since the last take. */
	uint32_t bank_reads[512];
};

size_t gcb_sizeof(void);
int gcb_init(struct gcb *c, const uint8_t *rom, size_t rom_len);
size_t gcb_save_size(struct gcb *c);
void gcb_attach_cart_ram(struct gcb *c, uint8_t *ram, size_t len);
void gcb_reset(struct gcb *c);
int gcb_run_frame(struct gcb *c);
uint16_t gcb_error_addr(const struct gcb *c);
const uint16_t *gcb_framebuffer(const struct gcb *c);
const uint8_t *gcb_framebuffer_index(const struct gcb *c);
struct gcb_mem gcb_mem(struct gcb *c);
struct gcb_state gcb_state(const struct gcb *c);
uint8_t gcb_read(struct gcb *c, uint16_t addr);
void gcb_write(struct gcb *c, uint16_t addr, uint8_t val);
void gcb_set_lcd(struct gcb *c, int draw);
void gcb_set_joypad(struct gcb *c, uint8_t pressed);
void gcb_get_rtc(const struct gcb *c, uint8_t out[5]);
void gcb_set_rtc(struct gcb *c, const uint8_t in[5]);
void gcb_take_stats(struct gcb *c, struct gcb_stats *out);

#endif
