#ifndef _LX_LINUX_WORKQUEUE_H
#define _LX_LINUX_WORKQUEUE_H
struct work_struct;
typedef void (*work_func_t)(struct work_struct *);
struct work_struct { work_func_t func; };
struct delayed_work { struct work_struct work; };
void lx_init_work(struct work_struct *w, work_func_t fn);
int lx_schedule_work(struct work_struct *w);
void lx_flush_work(struct work_struct *w);
#define INIT_WORK(w, fn) lx_init_work((w), (fn))
#define schedule_work(w) lx_schedule_work(w)
#define flush_work(w) lx_flush_work(w)
#define INIT_DELAYED_WORK(w, fn) lx_init_work(&(w)->work, (fn))
#define cancel_delayed_work_sync(w) (0)
#define cancel_work_sync(w) (0)
#endif
