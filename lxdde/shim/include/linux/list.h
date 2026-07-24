#ifndef _LX_LINUX_LIST_H
#define _LX_LINUX_LIST_H
#include <linux/types.h>
#include <linux/stddef.h>

#ifndef container_of
#define container_of(ptr, type, member) \
	((type *)((char *)(ptr) - offsetof(type, member)))
#endif

/* struct list_head lo define <linux/types.h> (árbol real). */
static inline void INIT_LIST_HEAD(struct list_head *l) { l->next = l; l->prev = l; }
static inline void __list_add(struct list_head *n, struct list_head *p, struct list_head *x)
{ x->prev = n; n->next = x; n->prev = p; p->next = n; }
static inline void list_add(struct list_head *n, struct list_head *h) { __list_add(n, h, h->next); }
static inline void list_add_tail(struct list_head *n, struct list_head *h) { __list_add(n, h->prev, h); }
static inline void list_del(struct list_head *e) { e->next->prev = e->prev; e->prev->next = e->next; e->next = e->prev = e; }
static inline void list_del_init(struct list_head *e) { list_del(e); INIT_LIST_HEAD(e); }
static inline void list_move(struct list_head *e, struct list_head *h) { list_del(e); list_add(e, h); }
static inline void list_move_tail(struct list_head *e, struct list_head *h) { list_del(e); list_add_tail(e, h); }
static inline int list_empty(const struct list_head *h) { return h->next == h; }
static inline int list_is_last(const struct list_head *e, const struct list_head *h) { return e->next == h; }

#define LIST_HEAD_INIT(name) { &(name), &(name) }
#define LIST_HEAD(name) struct list_head name = LIST_HEAD_INIT(name)
#define list_entry(ptr, type, member) container_of(ptr, type, member)
#define list_first_entry(ptr, type, member) list_entry((ptr)->next, type, member)
#define list_last_entry(ptr, type, member) list_entry((ptr)->prev, type, member)
#define list_next_entry(pos, member) list_entry((pos)->member.next, typeof(*(pos)), member)
#define list_prev_entry(pos, member) list_entry((pos)->member.prev, typeof(*(pos)), member)
#define list_first_entry_or_null(ptr, type, member) (list_empty(ptr) ? NULL : list_first_entry(ptr, type, member))
#define list_for_each_entry(pos, head, member) \
	for (pos = list_first_entry(head, typeof(*pos), member); \
	     &pos->member != (head); pos = list_next_entry(pos, member))
#define list_for_each_entry_safe(pos, n, head, member) \
	for (pos = list_first_entry(head, typeof(*pos), member), \
	     n = list_next_entry(pos, member); &pos->member != (head); \
	     pos = n, n = list_next_entry(n, member))
#define list_for_each_entry_continue(pos, head, member) \
	for (pos = list_next_entry(pos, member); &pos->member != (head); pos = list_next_entry(pos, member))
#define list_for_each_entry_reverse(pos, head, member) \
	for (pos = list_last_entry(head, typeof(*pos), member); \
	     &pos->member != (head); pos = list_prev_entry(pos, member))
#define list_for_each_entry_continue_reverse(pos, head, member) \
	for (pos = list_prev_entry(pos, member); &pos->member != (head); pos = list_prev_entry(pos, member))
#define list_for_each_entry_safe_reverse(pos, n, head, member) \
	for (pos = list_last_entry(head, typeof(*pos), member), n = list_prev_entry(pos, member); \
	     &pos->member != (head); pos = n, n = list_prev_entry(n, member))
#endif
