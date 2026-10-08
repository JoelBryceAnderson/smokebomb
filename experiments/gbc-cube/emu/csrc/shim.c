/*
 * A small, stable C ABI over walnut-cgb for the Rust side.
 *
 * walnut-cgb is a single-header emulator: the header holds the whole core,
 * and its `struct gb_s` layout depends on the config macros. Keeping every
 * access to that struct in C means Rust never has to mirror it.
 *
 * ROM reads stay in C as well (they run millions of times a second): the
 * ROM is one flat buffer owned by Rust. On hardware this is where the
 * external-flash bank cache would sit; the read callbacks count bank
 * switches so the simulator can measure what that cache would see.
 */

#ifndef GCB_NO_SETJMP
#include <setjmp.h>
#endif
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define ENABLE_SOUND 0
#define ENABLE_LCD 1
/* The header prototypes a few functions on `struct gb_s` before declaring
 * the struct; C needs it declared first. */
struct gb_s;
#include "walnut_cgb.h"

#include "shim.h"

struct gcb {
	struct gb_s gb;
	const uint8_t *rom;
	size_t rom_len;
	uint8_t *cart_ram;
	size_t cart_ram_len;
	/* RGB565, native endian, row-major 160x144. */
	uint16_t fb[GCB_LCD_HEIGHT * GCB_LCD_WIDTH];
	/* Palette index per pixel as the PPU produced it (0x00-0x1F BG,
	 * 0x20-0x3F OBJ), for debugging. */
	uint8_t fb_index[GCB_LCD_HEIGHT * GCB_LCD_WIDTH];
#ifndef GCB_NO_SETJMP
	jmp_buf on_error;
#endif
	int error;
	uint16_t error_addr;
	struct gcb_stats stats;
	uint16_t last_bank;
};

#define GCB(gb) ((struct gcb *)(gb)->direct.priv)

static inline void note_bank(struct gcb *c, uint_fast32_t addr)
{
	uint16_t bank = (uint16_t)(addr >> 14);
	c->stats.rom_reads++;
	if (bank != c->last_bank) {
		c->last_bank = bank;
		c->stats.bank_changes++;
	}
	c->stats.banks_touched[(bank >> 3) & 63] |= (uint8_t)(1u << (bank & 7));
	c->stats.bank_reads[bank & 511]++;
}

static uint8_t rom_read(struct gb_s *gb, const uint_fast32_t addr)
{
	struct gcb *c = GCB(gb);
	note_bank(c, addr);
	return addr < c->rom_len ? c->rom[addr] : 0xFF;
}

static uint16_t rom_read16(struct gb_s *gb, const uint_fast32_t addr)
{
	struct gcb *c = GCB(gb);
	note_bank(c, addr);
	if (addr + 1 < c->rom_len) {
		uint16_t v;
		memcpy(&v, c->rom + addr, 2);
		return v;
	}
	return 0xFFFF;
}

static uint32_t rom_read32(struct gb_s *gb, const uint_fast32_t addr)
{
	struct gcb *c = GCB(gb);
	note_bank(c, addr);
	if (addr + 3 < c->rom_len) {
		uint32_t v;
		memcpy(&v, c->rom + addr, 4);
		return v;
	}
	return 0xFFFFFFFF;
}

static uint8_t cart_ram_read(struct gb_s *gb, const uint_fast32_t addr)
{
	struct gcb *c = GCB(gb);
	return addr < c->cart_ram_len ? c->cart_ram[addr] : 0xFF;
}

static void cart_ram_write(struct gb_s *gb, const uint_fast32_t addr, const uint8_t val)
{
	struct gcb *c = GCB(gb);
	if (addr < c->cart_ram_len) {
		c->cart_ram[addr] = val;
		c->stats.cart_ram_writes++;
	}
}

/* walnut-cgb must not return from its error callback: unwind to the
 * gcb_run_frame that called into the core. (Bare-metal benchmark builds
 * without setjmp stop instead.) */
static void on_error(struct gb_s *gb, const enum gb_error_e err, const uint16_t addr)
{
	struct gcb *c = GCB(gb);
	c->error = (int)err;
	c->error_addr = addr;
#ifndef GCB_NO_SETJMP
	longjmp(c->on_error, 1);
#else
	abort();
#endif
}

static void draw_line(struct gb_s *gb, const uint8_t *pixels, const uint_fast8_t line)
{
	struct gcb *c = GCB(gb);
	if (line >= GCB_LCD_HEIGHT)
		return;
	uint16_t *row = &c->fb[line * GCB_LCD_WIDTH];
	uint8_t *idx = &c->fb_index[line * GCB_LCD_WIDTH];
	if (gb->cgb.cgbMode) {
		for (int x = 0; x < GCB_LCD_WIDTH; x++) {
			idx[x] = pixels[x] & 0x3F;
			row[x] = gb->cgb.fixPalette[pixels[x] & 0x3F];
		}
	} else {
		/* DMG game: plain four greys (Crystal never takes this path). */
		static const uint16_t grey[4] = { 0xFFFF, 0xAD55, 0x52AA, 0x0000 };
		for (int x = 0; x < GCB_LCD_WIDTH; x++) {
			idx[x] = pixels[x];
			row[x] = grey[pixels[x] & 3];
		}
	}
}

size_t gcb_sizeof(void)
{
	return sizeof(struct gcb);
}

int gcb_init(struct gcb *c, const uint8_t *rom, size_t rom_len)
{
	memset(c, 0, sizeof(*c));
	c->rom = rom;
	c->rom_len = rom_len;
#ifndef GCB_NO_SETJMP
	if (setjmp(c->on_error))
		return 100 + c->error;
#endif
	int r = (int)gb_init(&c->gb, rom_read, rom_read16, rom_read32, cart_ram_read,
			     cart_ram_write, on_error, c);
	if (r != 0)
		return r;
	gb_init_lcd(&c->gb, draw_line);
	return 0;
}

size_t gcb_save_size(struct gcb *c)
{
	size_t size = 0;
	if (gb_get_save_size_s(&c->gb, &size) != 0)
		return 0;
	return size;
}

void gcb_attach_cart_ram(struct gcb *c, uint8_t *ram, size_t len)
{
	c->cart_ram = ram;
	c->cart_ram_len = len;
}

void gcb_reset(struct gcb *c)
{
	gb_reset(&c->gb);
}

int gcb_run_frame(struct gcb *c)
{
#ifndef GCB_NO_SETJMP
	if (setjmp(c->on_error))
		return c->error ? c->error : -1;
#endif
	gb_run_frame_dualfetch(&c->gb);
	c->stats.frames++;
	return 0;
}

uint16_t gcb_error_addr(const struct gcb *c)
{
	return c->error_addr;
}

const uint16_t *gcb_framebuffer(const struct gcb *c)
{
	return c->fb;
}

const uint8_t *gcb_framebuffer_index(const struct gcb *c)
{
	return c->fb_index;
}

struct gcb_mem gcb_mem(struct gcb *c)
{
	struct gcb_mem m;
	m.wram = c->gb.wram;
	m.wram_len = sizeof(c->gb.wram);
	m.vram = c->gb.vram;
	m.vram_len = sizeof(c->gb.vram);
	m.oam = c->gb.oam;
	m.hram_io = c->gb.hram_io;
	m.bg_palette = c->gb.cgb.BGPalette;
	m.obj_palette = c->gb.cgb.OAMPalette;
	m.fix_palette = c->gb.cgb.fixPalette;
	return m;
}

struct gcb_state gcb_state(const struct gcb *c)
{
	struct gcb_state s;
	s.cgb_mode = c->gb.cgb.cgbMode;
	s.double_speed = c->gb.cgb.doubleSpeed;
	s.halted = c->gb.gb_halt;
	s.wram_bank = c->gb.cgb.wramBank;
	s.vram_bank = c->gb.cgb.vramBank;
	s.rom_bank = c->gb.selected_rom_bank;
	s.pc = c->gb.cpu_reg.pc.reg;
	s.sp = c->gb.cpu_reg.sp.reg;
	return s;
}

uint8_t gcb_read(struct gcb *c, uint16_t addr)
{
	return __gb_read(&c->gb, addr);
}

void gcb_write(struct gcb *c, uint16_t addr, uint8_t val)
{
	__gb_write(&c->gb, addr, val);
}

void gcb_set_lcd(struct gcb *c, int draw)
{
	/* walnut-cgb skips line rendering (but keeps LCD timing, interrupts
	 * and STAT) while there's no line callback. */
	c->gb.display.lcd_draw_line = draw ? draw_line : NULL;
}

void gcb_set_joypad(struct gcb *c, uint8_t pressed)
{
	/* walnut-cgb's joypad byte is active-low. */
	c->gb.direct.joypad = (uint8_t)~pressed;
}

void gcb_get_rtc(const struct gcb *c, uint8_t out[5])
{
	memcpy(out, c->gb.rtc_real.bytes, 5);
}

void gcb_set_rtc(struct gcb *c, const uint8_t in[5])
{
	memcpy(c->gb.rtc_real.bytes, in, 5);
	memcpy(c->gb.rtc_latched.bytes, in, 5);
}

void gcb_take_stats(struct gcb *c, struct gcb_stats *out)
{
	*out = c->stats;
	memset(&c->stats, 0, sizeof(c->stats));
}
