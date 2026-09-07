#ifndef GSP_FWSEC_H
#define GSP_FWSEC_H

#include <stdint.h>

/* FWSEC-FRTS desde VBIOS: monta WPR2 antes del booter_load en SEC2 (tu102_gsp_init). */
int gsp_fwsec_run_frts(uint64_t frts_addr, uint64_t frts_size);

/* Parseo + parche (sin arrancar falcon). Para hostcheck y diagnóstico. */
int gsp_fwsec_probe(uint64_t frts_addr, uint64_t frts_size);

/* 0 si WPR2 está programado (NV_PFB_PRI_MMU_WPR2_ADDR_HI ≠ 0). */
int gsp_fwsec_wpr2_present(uint64_t *lo_out, uint64_t *hi_out);

/* 1 si el último parche FRTS usó la cabecera appif v1 (no el fallback DMAP). */
int gsp_fwsec_patch_via_appif(void);

#endif
