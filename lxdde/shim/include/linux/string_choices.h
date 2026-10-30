#ifndef _LX_LINUX_STRING_CHOICES_H_
#define _LX_LINUX_STRING_CHOICES_H_

#include <stdbool.h>

static inline const char *str_yes_no(bool v)
{
	return v ? "yes" : "no";
}

#endif
