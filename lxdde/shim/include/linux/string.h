#ifndef _LX_LINUX_STRING_H
#define _LX_LINUX_STRING_H
#include <linux/types.h>
void *memcpy(void *, const void *, size_t);
void *memset(void *, int, size_t);
void *memmove(void *, const void *, size_t);
int memcmp(const void *, const void *, size_t);
size_t strlen(const char *);
int strcmp(const char *, const char *);
int strncmp(const char *, const char *, size_t);
static inline int __lx_lc(int c) { return (c >= 'A' && c <= 'Z') ? c + 32 : c; }
static inline int strcasecmp(const char *a, const char *b)
{ for (; *a && __lx_lc(*a) == __lx_lc(*b); a++, b++) ; return __lx_lc((unsigned char)*a) - __lx_lc((unsigned char)*b); }
static inline int strncasecmp(const char *a, const char *b, size_t n)
{ for (; n && *a && __lx_lc(*a) == __lx_lc(*b); a++, b++, n--) ; return n ? __lx_lc((unsigned char)*a) - __lx_lc((unsigned char)*b) : 0; }
char *strcpy(char *, const char *);
char *strncpy(char *, const char *, size_t);
int snprintf(char *, size_t, const char *, ...);
int scnprintf(char *, size_t, const char *, ...);

static inline size_t strcspn(const char *s, const char *reject)
{
	size_t n = 0;
	for (; s[n]; n++) {
		const char *r;
		for (r = reject; *r; r++)
			if (s[n] == *r)
				return n;
	}
	return n;
}

static inline int kstrtol(const char *s, unsigned int base, long *res)
{
	long v = 0;
	int neg = 0;
	if (*s == '-') { neg = 1; s++; } else if (*s == '+') { s++; }
	if (base == 0)
		base = 10;
	for (; *s && *s != '\n'; s++) {
		int d;
		if (*s >= '0' && *s <= '9') d = *s - '0';
		else if (*s >= 'a' && *s <= 'f') d = *s - 'a' + 10;
		else if (*s >= 'A' && *s <= 'F') d = *s - 'A' + 10;
		else return -22; /* -EINVAL */
		if ((unsigned)d >= base) return -22;
		v = v * base + d;
	}
	*res = neg ? -v : v;
	return 0;
}

static inline int kstrtoul(const char *s, unsigned int base, unsigned long *res)
{
	long v;
	int r = kstrtol(s, base, &v);
	if (r) return r;
	*res = (unsigned long)v;
	return 0;
}

static inline long strscpy(char *dst, const char *src, size_t count)
{
	size_t i = 0;
	if (!count) return -7; /* -E2BIG */
	for (; i < count - 1 && src[i]; i++) dst[i] = src[i];
	dst[i] = '\0';
	return src[i] ? -7 : (long)i;
}
#endif
