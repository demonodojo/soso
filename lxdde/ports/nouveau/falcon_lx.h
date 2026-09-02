#ifndef FALCON_LX_H
#define FALCON_LX_H

#include "acr_fw.h"

/* Bases BAR0 ga102 (referencia GB205 hasta calibrar TOP en placa). */
#define LX_FLCN_SEC2_BASE 0x00840000u
#define LX_FLCN_GSP_BASE  0x00110000u
#define LX_FLCN_ADDR2     0x00001000u

int falcon_lx_hsfw_boot(unsigned falcon_base, const struct acr_fw_blob *blob, const char *name);
/* Igual, pero el mailbox lo pone el llamante (booter_load: física del WPR meta).
 * `check_mbox0 == 0` no exige un valor concreto al halt. */
int falcon_lx_hsfw_boot_mbox(unsigned falcon_base, const struct acr_fw_blob *blob,
                             const char *name, unsigned mbox0, unsigned mbox1,
                             int check_mbox0);

#endif
