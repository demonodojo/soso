#ifndef _LX_LINUX_LIST_H
#define _LX_LINUX_LIST_H
#include <linux/types.h>
#include <linux/stddef.h>
#ifndef container_of
#define container_of(ptr, type, member) \
	((type *)((char *)(ptr) - offsetof(type, member)))
#endif
struct list_head { struct list_head *next, *prev; };
static inline void INIT_LIST_HEAD(struct list_head *l) { l->next = l; l->prev = l; }
static inline void __list_add(struct list_head *n, struct list_head *p, struct list_head *x)
{ x->prev = n; n->next = x; n->prev = p; p->next = n; }
static inline void list_add(struct list_head *n, struct list_head *h) { __list_add(n, h, h->next); }
static inline void list_add_tail(struct list_head *n, struct list_head *h) { __list_add(n, h->prev, h); }
static inline void list_del(struct list_head *e) { e->next->prev = e->prev; e->prev->next = e->next; e->next = e->prev = e; }
static inline int list_empty(const struct list_head *h) { return h->next == h; }
#define LIST_HEAD_INIT(name) { &(name), &(name) }
#define LIST_HEAD(name) struct list_head name = LIST_HEAD_INIT(name)
#define list_entry(ptr, type, member) container_of(ptr, type, member)
#define list_first_entry(ptr, type, member) list_entry((ptr)->next, type, member)
#define list_next_entry(pos, member) list_entry((pos)->member.next, typeof(*(pos)), member)
#define list_for_each_entry(pos, head, member) \
	for (pos = list_first_entry(head, typeof(*pos), member); \
	     &pos->member != (head); pos = list_next_entry(pos, member))
#define list_for_each_entry_safe(pos, n, head, member) \
	for (pos = list_first_entry(head, typeof(*pos), member), \
	     n = list_next_entry(pos, member); &pos->member != (head); \
	     pos = n, n = list_next_entry(n, member))
#define list_for_each_entry_continue(pos, head, member) \
	for (pos = list_next_entry(pos, member); &pos->member != (head); pos = list_next_entry(pos, member))
#endif
