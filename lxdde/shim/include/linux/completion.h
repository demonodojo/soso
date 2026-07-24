#ifndef _LX_LINUX_COMPLETION_H
#define _LX_LINUX_COMPLETION_H
struct completion { unsigned int done; };
void lx_init_completion(struct completion *c);
void lx_complete(struct completion *c);
int lx_wait_for_completion(struct completion *c);
int lx_wait_for_completion_timeout(struct completion *c, unsigned long ms);
#define init_completion(c) lx_init_completion(c)
#define reinit_completion(c) lx_init_completion(c)
#define complete(c) lx_complete(c)
#define complete_all(c) lx_complete(c)
#define wait_for_completion(c) lx_wait_for_completion(c)
#define wait_for_completion_timeout(c, t) lx_wait_for_completion_timeout((c), (t))
#endif
