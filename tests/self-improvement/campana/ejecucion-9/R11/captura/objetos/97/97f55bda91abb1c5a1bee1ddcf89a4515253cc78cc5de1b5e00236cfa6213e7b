#ifndef _LX_LINUX_RCUPDATE_H
#define _LX_LINUX_RCUPDATE_H
/* Monocore: RCU es no-op. */
#ifndef __rcu
#define __rcu
#endif
#define rcu_assign_pointer(p, v) ((p) = (typeof(p))(v))
#define RCU_INIT_POINTER(p, v)   ((p) = (typeof(p))(v))
#define rcu_dereference(p)       (p)
#define rcu_dereference_raw(p)   (p)
#define rcu_dereference_protected(p, c) (p)
#define rcu_read_lock()          do {} while (0)
#define rcu_read_unlock()        do {} while (0)
#define synchronize_rcu()        do {} while (0)
#define call_rcu(h, f)           do {} while (0)
#endif
