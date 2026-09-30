/*
 * The few Zephyr calls the Rust firmware makes. Most of Zephyr's API is
 * inline functions and macros, which Rust can't call, so each is wrapped
 * here as a plain function.
 */
#include <stddef.h>
#include <stdint.h>

#include <errno.h>

#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

uint64_t sb_uptime_us(void)
{
	return k_ticks_to_us_floor64(k_uptime_ticks());
}

void sb_sleep_us(uint32_t us)
{
	k_usleep((int32_t)us);
}

void sb_print(const char *s, size_t len)
{
	printk("%.*s", (int)len, s);
}

FUNC_NORETURN void sb_panic(void)
{
	k_panic();
	for (;;) {
	}
}

/*
 * Stands in for a driver the board HAL doesn't have yet: always fails. The
 * Rust side asks here rather than failing on its own so the compiler can't
 * drop the rest of the firmware as unreachable.
 */
int sb_hal_todo(void)
{
	return -ENOSYS;
}
