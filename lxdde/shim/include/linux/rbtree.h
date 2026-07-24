/* Shim mínimo de <linux/rbtree.h>: structs reales + helpers inline.
 * Los algoritmos (rb_insert_color/rb_erase/rb_first/rb_next) quedan declarados;
 * si se referencian sin portar lib/rbtree.c reciben dummy trace_and_stop.
 * Suficiente para que las structs nvkm que embeben rb_node/rb_root compilen.
 */
#ifndef _LX_LINUX_RBTREE_H
#define _LX_LINUX_RBTREE_H
#include <linux/types.h>
#include <linux/stddef.h>

struct rb_node {
	unsigned long __rb_parent_color;
	struct rb_node *rb_right;
	struct rb_node *rb_left;
};

struct rb_root {
	struct rb_node *rb_node;
};

struct rb_root_cached {
	struct rb_root rb_root;
	struct rb_node *rb_leftmost;
};

#define RB_ROOT (struct rb_root){ NULL, }
#define RB_ROOT_CACHED (struct rb_root_cached){ { NULL, }, NULL }
#define rb_entry(ptr, type, member) container_of(ptr, type, member)
#define rb_entry_safe(ptr, type, member) \
	({ typeof(ptr) ____ptr = (ptr); ____ptr ? rb_entry(____ptr, type, member) : NULL; })

#define RB_EMPTY_ROOT(root) ((root)->rb_node == NULL)
#define RB_EMPTY_NODE(node) ((node)->__rb_parent_color == (unsigned long)(node))
#define RB_CLEAR_NODE(node) ((node)->__rb_parent_color = (unsigned long)(node))

static inline void rb_link_node(struct rb_node *node, struct rb_node *parent,
				struct rb_node **rb_link)
{
	node->__rb_parent_color = (unsigned long)parent;
	node->rb_left = node->rb_right = NULL;
	*rb_link = node;
}

void rb_insert_color(struct rb_node *, struct rb_root *);
void rb_erase(struct rb_node *, struct rb_root *);
struct rb_node *rb_next(const struct rb_node *);
struct rb_node *rb_prev(const struct rb_node *);
struct rb_node *rb_first(const struct rb_root *);
struct rb_node *rb_last(const struct rb_root *);
void rb_insert_color_cached(struct rb_node *, struct rb_root_cached *, bool);
void rb_erase_cached(struct rb_node *, struct rb_root_cached *);

#endif /* _LX_LINUX_RBTREE_H */
