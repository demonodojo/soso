#ifndef _LINUX_TIMECOUNTER_H
#define _LINUX_TIMECOUNTER_H
#include <linux/ktime.h>
struct cyclecounter { u64 (*read)(const struct cyclecounter *); u64 mask; u32 shift; };
struct timecounter { struct cyclecounter *cc; u64 cycle_last; u64 nsec; };
static inline void timecounter_init(struct timecounter *tc, struct cyclecounter *cc, u64 start) {
	(void)tc; (void)cc; (void)start;
}
#endif
