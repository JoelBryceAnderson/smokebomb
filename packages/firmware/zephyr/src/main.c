/*
 * Zephyr entry point. All application logic lives in Rust; see
 * packages/firmware/src/lib.rs (smokebomb_main).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

extern int smokebomb_main(void);

int main(void)
{
	int err = smokebomb_main();

	printk("smokebomb_main returned %d\n", err);
	return err;
}
