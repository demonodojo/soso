#ifndef _LX_LINUX_MUTEX_H
#define _LX_LINUX_MUTEX_H
struct mutex { int locked; };
static inline void mutex_init(struct mutex *m) { m->locked = 0; }
static inline void mutex_lock(struct mutex *m) { m->locked = 1; }
static inline void mutex_unlock(struct mutex *m) { m->locked = 0; }
static inline int mutex_trylock(struct mutex *m) { m->locked = 1; return 1; }
static inline int mutex_is_locked(struct mutex *m) { return m->locked; }
#define DEFINE_MUTEX(name) struct mutex name = { 0 }
#endif
