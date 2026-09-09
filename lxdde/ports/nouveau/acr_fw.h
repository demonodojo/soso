#ifndef ACR_FW_H
#define ACR_FW_H

#include <stdint.h>

enum acr_fw_kind {
    ACR_FW_AHESASC = 0,
    ACR_FW_ASB,
    ACR_FW_COUNT,
};

struct acr_fw_blob {
    const char *path;
    unsigned char *data;
    unsigned long len;
    unsigned long payload_len;
    uint64_t dma_handle;
    unsigned char *dma_cpu;
    int valid;
};

int acr_fw_load_all(void);
const struct acr_fw_blob *acr_fw_get(enum acr_fw_kind kind);
void acr_fw_release_all(void);

#endif
