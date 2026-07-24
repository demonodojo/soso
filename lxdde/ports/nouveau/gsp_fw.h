/* G3: catálogo de blobs GSP gb205 (copias persistentes en lx heap). */
#ifndef GSP_FW_H
#define GSP_FW_H

enum gsp_fw_kind {
    GSP_FW_BOOTLOADER = 0,
    GSP_FW_FMC,
    GSP_FW_UCODE,
    GSP_FW_GA102_UCODE,
    GSP_FW_COUNT,
};

struct gsp_fw_blob {
    const char *path;
    unsigned char *data;
    unsigned long len;
    unsigned gem_handle;
    int valid;
};

int gsp_fw_load_all(void);
const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind kind);
int gsp_fw_stage_all(void);
void gsp_fw_release_all(void);

#endif
