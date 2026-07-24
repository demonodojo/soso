#ifndef _LX_LINUX_BUG_H
#define _LX_LINUX_BUG_H
#define BUG() do { } while (1)
#define BUG_ON(c) do { if (c) BUG(); } while (0)
#define WARN_ON(c) ({ int __c = !!(c); __c; })
#define WARN_ON_ONCE(c) WARN_ON(c)
#define WARN(c, ...) ({ int __c = !!(c); __c; })
#define WARN_ONCE(c, ...) WARN(c, __VA_ARGS__)
#endif
