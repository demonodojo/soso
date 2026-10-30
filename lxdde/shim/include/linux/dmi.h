/* stub lx_emul: DMI desactivado en bare-metal (gpio Mac quirk nunca aplica). */
#ifndef _LX_LINUX_DMI_H
#define _LX_LINUX_DMI_H

#define DMI_SYS_VENDOR 1
#define DMI_PRODUCT_NAME 2

struct dmi_strmatch {
	unsigned char slot;
	char substr[79];
};

struct dmi_system_id {
	const char *ident;
	struct {
		unsigned char slot;
		char substr[79];
	} matches[4];
};

#define DMI_MATCH(_slot, _substr) \
	{ .slot = (_slot), .substr = (_substr) }

static inline int dmi_check_system(const struct dmi_system_id *list)
{
	(void)list;
	return 0;
}

#endif
