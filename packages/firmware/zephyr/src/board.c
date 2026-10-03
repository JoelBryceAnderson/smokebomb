/*
 * The board's devices, as plain functions for the Rust HAL
 * (packages/firmware/hal/nrf54l15). Each part is found by its devicetree
 * label (the overlays in boards/); on a board without it, its functions
 * fail with -ENODEV so the firmware still links and says what's missing.
 *
 * Every function returns 0 or a negative errno.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include <errno.h>

#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/haptics.h>
#include <zephyr/drivers/haptics/drv2605.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/sensor.h>
#include <zephyr/drivers/spi.h>
#include <zephyr/input/input.h>
#include <zephyr/sys/atomic.h>
#include <zephyr/sys/util.h>

/* Panels: six SSD1317s on one SPI bus, faceN for Face index N. */

#define FACE(n) DT_NODELABEL(face##n)

#if DT_NODE_HAS_STATUS_OKAY(FACE(0))
#define PANEL_SPI(n) SPI_DT_SPEC_GET(FACE(n), SPI_WORD_SET(8) | SPI_TRANSFER_MSB, 0)

static const struct spi_dt_spec panels[] = {
	PANEL_SPI(0), PANEL_SPI(1), PANEL_SPI(2), PANEL_SPI(3), PANEL_SPI(4), PANEL_SPI(5),
};
static const struct gpio_dt_spec panel_dc = GPIO_DT_SPEC_GET(FACE(0), dc_gpios);
static const struct gpio_dt_spec panel_reset = GPIO_DT_SPEC_GET(FACE(0), reset_gpios);

int sb_panel_setup(void)
{
	for (size_t i = 0; i < ARRAY_SIZE(panels); i++) {
		if (!spi_is_ready_dt(&panels[i])) {
			return -ENODEV;
		}
	}
	if (!gpio_is_ready_dt(&panel_dc) || !gpio_is_ready_dt(&panel_reset)) {
		return -ENODEV;
	}
	int err = gpio_pin_configure_dt(&panel_dc, GPIO_OUTPUT_INACTIVE);

	if (err) {
		return err;
	}
	return gpio_pin_configure_dt(&panel_reset, GPIO_OUTPUT_INACTIVE);
}

int sb_panel_write(uint8_t face, bool data, const uint8_t *buf, size_t len)
{
	if (face >= ARRAY_SIZE(panels)) {
		return -EINVAL;
	}
	int err = gpio_pin_set_dt(&panel_dc, data);

	if (err) {
		return err;
	}
	const struct spi_buf tx = {.buf = (void *)buf, .len = len};
	const struct spi_buf_set set = {.buffers = &tx, .count = 1};

	return spi_write_dt(&panels[face], &set);
}

int sb_panel_reset(bool asserted)
{
	return gpio_pin_set_dt(&panel_reset, asserted);
}
#else
int sb_panel_setup(void)
{
	return -ENODEV;
}

int sb_panel_write(uint8_t face, bool data, const uint8_t *buf, size_t len)
{
	return -ENODEV;
}

int sb_panel_reset(bool asserted)
{
	return -ENODEV;
}
#endif

/* IMU: Zephyr's sensor API, in the units the HAL's ImuSample uses. */

#if DT_NODE_HAS_STATUS_OKAY(DT_NODELABEL(imu))
static const struct device *const imu = DEVICE_DT_GET(DT_NODELABEL(imu));

int sb_imu_read(int16_t accel_mg[3], int32_t gyro_mdps[3])
{
	struct sensor_value v[3];

	if (!device_is_ready(imu)) {
		return -ENODEV;
	}
	int err = sensor_sample_fetch(imu);

	if (!err) {
		err = sensor_channel_get(imu, SENSOR_CHAN_ACCEL_XYZ, v);
	}
	if (err) {
		return err;
	}
	for (int i = 0; i < 3; i++) {
		/* m/s² to milli-g */
		accel_mg[i] = (int16_t)(sensor_value_to_micro(&v[i]) / 9807);
	}
	err = sensor_channel_get(imu, SENSOR_CHAN_GYRO_XYZ, v);
	if (err) {
		return err;
	}
	for (int i = 0; i < 3; i++) {
		/* rad/s to milli-degrees a second: 1 rad = 57295.78 mdeg */
		gyro_mdps[i] = (int32_t)(sensor_value_to_micro(&v[i]) * 57296 / 1000000);
	}
	return 0;
}
#else
int sb_imu_read(int16_t accel_mg[3], int32_t gyro_mdps[3])
{
	return -ENODEV;
}
#endif

/* Touch: the CAP12xx input driver reports pad N as key N; keep a bitmask. */

static atomic_t touched;

#if DT_NODE_HAS_STATUS_OKAY(DT_NODELABEL(touch))
static const struct device *const touch = DEVICE_DT_GET(DT_NODELABEL(touch));

static void touch_event(struct input_event *evt, void *user_data)
{
	ARG_UNUSED(user_data);
	if (evt->type != INPUT_EV_KEY || evt->code < INPUT_KEY_0 || evt->code > INPUT_KEY_5) {
		return;
	}
	int bit = evt->code - INPUT_KEY_0;

	if (evt->value) {
		atomic_set_bit(&touched, bit);
	} else {
		atomic_clear_bit(&touched, bit);
	}
}
INPUT_CALLBACK_DEFINE(touch, touch_event, NULL);

int sb_touch_read(uint8_t *mask)
{
	if (!device_is_ready(touch)) {
		return -ENODEV;
	}
	*mask = (uint8_t)atomic_get(&touched);
	return 0;
}
#else
int sb_touch_read(uint8_t *mask)
{
	return -ENODEV;
}
#endif

/* Haptics: one effect from the DRV2605L's LRA library. */

#if DT_NODE_HAS_STATUS_OKAY(DT_NODELABEL(haptics))
static const struct device *const haptics = DEVICE_DT_GET(DT_NODELABEL(haptics));

int sb_haptics_play(uint8_t effect)
{
	struct drv2605_rom_data rom = {
		.trigger = DRV2605_MODE_INTERNAL_TRIGGER,
		.library = DRV2605_LIBRARY_LRA,
		.seq_regs = {effect},
	};
	union drv2605_config_data config = {.rom_data = &rom};

	if (!device_is_ready(haptics)) {
		return -ENODEV;
	}
	int err = drv2605_haptic_config(haptics, DRV2605_HAPTICS_SOURCE_ROM, &config);

	if (err) {
		return err;
	}
	return haptics_start_output(haptics);
}
#else
int sb_haptics_play(uint8_t effect)
{
	return -ENODEV;
}
#endif

/* The STEMMA chain, for the bring-up image: raw register access. */

#if DT_NODE_HAS_STATUS_OKAY(DT_NODELABEL(i2c22))
static const struct device *const stemma = DEVICE_DT_GET(DT_NODELABEL(i2c22));

/* Does anything answer at `addr`? A one-byte read, since the TWIM can't
 * send an empty write.
 */
int sb_i2c_probe(uint8_t addr)
{
	uint8_t b;

	if (!device_is_ready(stemma)) {
		return -ENODEV;
	}
	return i2c_read(stemma, &b, 1, addr);
}

int sb_i2c_reg_read(uint8_t addr, uint8_t reg, uint8_t *val)
{
	if (!device_is_ready(stemma)) {
		return -ENODEV;
	}
	return i2c_reg_read_byte(stemma, addr, reg, val);
}
#else
int sb_i2c_probe(uint8_t addr)
{
	return -ENODEV;
}

int sb_i2c_reg_read(uint8_t addr, uint8_t reg, uint8_t *val)
{
	return -ENODEV;
}
#endif

/* Which of the devicetree's devices came up. For the bring-up image. */

#define READY(label)                                                                               \
	COND_CODE_1(DT_NODE_HAS_STATUS_OKAY(DT_NODELABEL(label)),                                  \
		    (device_is_ready(DEVICE_DT_GET(DT_NODELABEL(label))) ? 1 : 0), (-1))

/* 1 ready, 0 failed its init, -1 not in this board's devicetree. */
int sb_device_ready(uint8_t which)
{
	switch (which) {
	case 0:
		return READY(imu);
	case 1:
		return READY(touch);
	case 2:
		return READY(haptics);
	case 3:
		return READY(mag_mmc5603);
	case 4:
		return READY(mag_lis2mdl);
	default:
		return -1;
	}
}
