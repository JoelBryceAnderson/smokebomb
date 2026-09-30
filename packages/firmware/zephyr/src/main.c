/*
 * Zephyr entry point. All application logic lives in Rust (the static
 * library built from packages/firmware/zephyr/rust); see
 * packages/firmware/src/lib.rs (smokebomb_main).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#ifdef CONFIG_SMOKEBOMB_BENCH
extern void smokebomb_bench(void);

int main(void)
{
	smokebomb_bench();
	return 0;
}
#else
extern int smokebomb_main(void);

int main(void)
{
	int err = smokebomb_main();

	printk("smokebomb_main returned %d\n", err);
	return err;
}
#endif
