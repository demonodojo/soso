#ifndef NVFW_LX_H
#define NVFW_LX_H

#include <stdint.h>

struct nvfw_bin_hdr {
    uint32_t bin_magic;
    uint32_t bin_ver;
    uint32_t bin_size;
    uint32_t header_offset;
    uint32_t data_offset;
    uint32_t data_size;
};

struct nvfw_hs_header_v2 {
    uint32_t sig_prod_offset;
    uint32_t sig_prod_size;
    uint32_t patch_loc;
    uint32_t patch_sig;
    uint32_t meta_data_offset;
    uint32_t meta_data_size;
    uint32_t num_sig;
    uint32_t header_offset;
    uint32_t header_size;
};

struct nvfw_hs_load_header_v2 {
    uint32_t os_code_offset;
    uint32_t os_code_size;
    uint32_t os_data_offset;
    uint32_t os_data_size;
    uint32_t num_apps;
    struct {
        uint32_t offset;
        uint32_t size;
    } app[1];
};

#define NVFW_BIN_MAGIC 0x000010deu

#endif
