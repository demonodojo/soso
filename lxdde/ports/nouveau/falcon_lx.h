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

struct falcon_lx_raw {
    const unsigned char *img;
    uint64_t dma_handle;
    unsigned imem_src;
    unsigned imem_dst;
    unsigned imem_len;
    unsigned dmem_src;
    unsigned dmem_dst;
    unsigned dmem_len;
    unsigned pkc_data_offset;
    unsigned engine_id_mask;
    unsigned ucode_id;
    unsigned boot_addr;
    unsigned mbox0;
    unsigned mbox1;
    int check_mbox0;
    unsigned timeout_ms;
    const char *name;
};

/* Reset del falcon antes de cada carga (ga102 HAL). */
int falcon_lx_reset(unsigned base);

/* SEC2 limpio antes del booter Ampere: reset + transcfg + DMA idle. */
int falcon_lx_sec2_prepare(unsigned sec2_base);

/* Fuse ucode (ga102 `flcn_read_fuse_reg`). */
uint32_t falcon_lx_read_fuse(unsigned engine_id, unsigned ucode_id);

/* Reset GSP para RISC-V (ga102_gsp_reset): no selecciona Falcon. */
int falcon_lx_gsp_reset_riscv(unsigned base);

/* Habilita el motor en PMC + espera mem scrub (gm200_flcn_enable). */
int falcon_lx_enable(unsigned falcon_base, uint8_t top_type, uint8_t top_inst);

/* Ucode crudo con parámetros BROM (FWSEC-FRTS en falcon GSP). */
int falcon_lx_raw_boot(unsigned falcon_base, const struct falcon_lx_raw *raw);

#endif
