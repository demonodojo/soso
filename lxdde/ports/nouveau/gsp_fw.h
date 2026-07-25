/* G3: catálogo de blobs GSP por familia (copias persistentes en lx heap). */
#ifndef GSP_FW_H
#define GSP_FW_H

enum gsp_fw_kind {
    GSP_FW_BOOTLOADER = 0,
    GSP_FW_FMC,             /* solo Blackwell (ruta FMC) */
    GSP_FW_UCODE,
    GSP_FW_BOOTER_LOAD,     /* solo Ampere */
    GSP_FW_BOOTER_UNLOAD,   /* solo Ampere */
    GSP_FW_COUNT,
};

/* Qué juego de blobs pedir. Lo decide el bring-up a partir de NV_PMC_BOOT_0. */
enum gsp_fw_chip {
    GSP_FW_CHIP_BLACKWELL = 0,
    GSP_FW_CHIP_AMPERE,
};

struct gsp_fw_blob {
    const char *path;
    unsigned char *data;
    unsigned long len;
    unsigned gem_handle;
    int valid;
};

int gsp_fw_load_all(enum gsp_fw_chip chip);
const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind kind);
int gsp_fw_stage_all(void);
void gsp_fw_release_all(void);

#endif
