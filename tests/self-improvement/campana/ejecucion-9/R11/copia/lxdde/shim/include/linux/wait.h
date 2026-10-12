#ifndef _LX_LINUX_WAIT_H
#define _LX_LINUX_WAIT_H
typedef struct wait_queue_head { int x; } wait_queue_head_t;
#define init_waitqueue_head(q) do { (q)->x = 0; } while (0)
#define wake_up(q) do { (void)(q); } while (0)
#define wake_up_all(q) do { (void)(q); } while (0)
#define wait_event(q, cond) do { (void)(q); } while (0)
#define wait_event_timeout(q, cond, to) ((cond) ? 1 : 0)
#endif
