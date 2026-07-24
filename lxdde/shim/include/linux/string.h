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
int strcasecmp(const char *, const char *);
int strncasecmp(const char *, const char *, size_t);
char *strcpy(char *, const char *);
char *strncpy(char *, const char *, size_t);
int snprintf(char *, size_t, const char *, ...);
int scnprintf(char *, size_t, const char *, ...);
#endif
