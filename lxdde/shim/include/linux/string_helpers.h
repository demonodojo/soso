#ifndef _LX_LINUX_STRING_HELPERS_H_
#define _LX_LINUX_STRING_HELPERS_H_

#include <linux/string.h>
#include <linux/types.h>
#include <stdbool.h>

static inline bool string_is_terminated(const char *s, int len)
{
	return memchr(s, '\0', len) ? true : false;
}

static inline const char *str_yes_no(bool v)
{
	return v ? "yes" : "no";
}

#endif
