#ifndef _LX_LINUX_SPINLOCK_H
#define _LX_LINUX_SPINLOCK_H
typedef struct { int lock; } spinlock_t;
typedef struct { int lock; } rwlock_t;
#define spin_lock_init(l) do { (l)->lock = 0; } while (0)
#define spin_lock(l) do { (void)(l); } while (0)
#define spin_unlock(l) do { (void)(l); } while (0)
#define spin_lock_irqsave(l, f) do { (void)(l); (f) = 0; } while (0)
#define spin_unlock_irqrestore(l, f) do { (void)(l); (void)(f); } while (0)
#define spin_lock_irq(l) do { (void)(l); } while (0)
#define spin_unlock_irq(l) do { (void)(l); } while (0)
#define rwlock_init(l) do { (l)->lock = 0; } while (0)
#define read_lock(l) do { (void)(l); } while (0)
#define read_unlock(l) do { (void)(l); } while (0)
#define write_lock(l) do { (void)(l); } while (0)
#define write_unlock(l) do { (void)(l); } while (0)
#define read_lock_irqsave(l, f) do { (void)(l); (f) = 0; } while (0)
#define read_unlock_irqrestore(l, f) do { (void)(l); (void)(f); } while (0)
#define write_lock_irq(l) do { (void)(l); } while (0)
#define write_unlock_irq(l) do { (void)(l); } while (0)
#define write_lock_irqsave(l, f) do { (void)(l); (f) = 0; } while (0)
#define write_unlock_irqrestore(l, f) do { (void)(l); (void)(f); } while (0)
#define assert_spin_locked(l) do { (void)(l); } while (0)
#define spin_lock_bh(l) do { (void)(l); } while (0)
#define spin_unlock_bh(l) do { (void)(l); } while (0)
#define DEFINE_SPINLOCK(name) spinlock_t name = { 0 }
#endif
