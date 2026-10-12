/* G3 ola 2: firmware ACR ga107/ga102 (AHESASC + ASB) en heap lx + DMA. */
#include "acr_fw.h"
#include "gsp_chip.h"
#include "nvfw_lx.h"
#include "lx_emul.h"

void *memcpy(void *dst, const void *src, unsigned long n);

static struct acr_fw_blob g_acr[ACR_FW_COUNT];
static int g_acr_loaded;
static char g_acr_paths[ACR_FW_COUNT][96];

static const char *const g_acr_leaf[ACR_FW_COUNT] = {
    "ucode_ahesasc.bin",
    "ucode_asb.bin",
};

static int acr_fw_stage_payload(struct acr_fw_blob *b, unsigned char *file_copy,
                                unsigned long file_len)
{
    const struct nvfw_bin_hdr *hdr = (const struct nvfw_bin_hdr *)file_copy;
    unsigned char *dma_cpu;
    uint64_t dma_handle = 0;

    if (file_len < sizeof(*hdr) || hdr->bin_magic != NVFW_BIN_MAGIC) {
        return -1;
    }
    if (hdr->data_offset + hdr->data_size > file_len || hdr->data_size == 0) {
        return -1;
    }

    dma_cpu = lx_dma_alloc_coherent(NULL, hdr->data_size, &dma_handle, GFP_KERNEL);
    if (!dma_cpu) {
        return -1;
    }
    memcpy(dma_cpu, file_copy + hdr->data_offset, hdr->data_size);

    b->data = file_copy;
    b->len = file_len;
    b->payload_len = hdr->data_size;
    b->dma_cpu = dma_cpu;
    b->dma_handle = dma_handle;
    b->valid = 1;
    lx_printk("nouveau-lx: acr fw %s (%lu bytes, payload=%lu dma=0x%llx)\n",
              b->path, file_len, b->payload_len,
              (unsigned long long)b->dma_handle);
    return 0;
}

static int load_one_path(enum acr_fw_kind kind, const char *path)
{
    const unsigned char *tmp = NULL;
    unsigned long tmp_len = 0;
    unsigned char *copy;
    struct acr_fw_blob *b = &g_acr[kind];

    if (lx_request_firmware(path, &tmp, &tmp_len) != 0 || !tmp || tmp_len < 64) {
        return -1;
    }
    copy = lx_kmalloc(tmp_len, GFP_KERNEL);
    if (!copy) {
        lx_release_firmware(tmp);
        return -1;
    }
    memcpy(copy, tmp, tmp_len);
    lx_release_firmware(tmp);

    b->path = path;
    if (acr_fw_stage_payload(b, copy, tmp_len) != 0) {
        lx_kfree(copy);
        return -1;
    }
    return 0;
}

static int load_acr_one(enum acr_fw_kind kind)
{
    const char *primary = gsp_nv_ampere_chip_name(gsp_nv_family_device_id());
    const char *fallback = "ga102";
    const char *chips[2];
    unsigned nchips = 0;
    char path[96];
    unsigned i;

    chips[nchips++] = primary;
    if (primary != fallback)
        chips[nchips++] = fallback;

    for (i = 0; i < nchips; i++) {
        const char *chip = chips[i];
        unsigned pos = 0;
        const char *prefix = "nvidia/";

        while (*prefix && pos + 1u < sizeof(path))
            path[pos++] = *prefix++;
        while (*chip && pos + 1u < sizeof(path))
            path[pos++] = *chip++;
        if (pos + 5u < sizeof(path)) {
            path[pos++] = '/';
            path[pos++] = 'a';
            path[pos++] = 'c';
            path[pos++] = 'r';
            path[pos++] = '/';
        }
        {
            const char *leaf = g_acr_leaf[kind];
            while (*leaf && pos + 1u < sizeof(path))
                path[pos++] = *leaf++;
            path[pos] = '\0';
        }
        if (load_one_path(kind, path) == 0) {
            unsigned j;
            char *stored = g_acr_paths[kind];

            for (j = 0; j < sizeof(path) && path[j]; j++)
                stored[j] = path[j];
            if (j < sizeof(g_acr_paths[0]))
                stored[j] = '\0';
            g_acr[kind].path = stored;
            return 0;
        }
    }
    return -1;
}

int acr_fw_load_all(void)
{
    unsigned i;
    int ok = 0;

    if (g_acr_loaded) {
        return 0;
    }
    for (i = 0; i < ACR_FW_COUNT; i++) {
        g_acr[i].valid = 0;
    }
    for (i = 0; i < ACR_FW_COUNT; i++) {
        if (load_acr_one((enum acr_fw_kind)i) == 0) {
            ok++;
        }
    }
    if (ok < 2) {
        lx_printk("nouveau-lx: ACR firmware incompleto (%d/2)\n", ok);
        acr_fw_release_all();
        return -1;
    }
    g_acr_loaded = 1;
    return 0;
}

const struct acr_fw_blob *acr_fw_get(enum acr_fw_kind kind)
{
    if (!g_acr_loaded || kind >= ACR_FW_COUNT || !g_acr[kind].valid) {
        return NULL;
    }
    return &g_acr[kind];
}

void acr_fw_release_all(void)
{
    unsigned i;
    for (i = 0; i < ACR_FW_COUNT; i++) {
        if (g_acr[i].dma_cpu) {
            lx_dma_free_coherent(NULL, g_acr[i].payload_len, g_acr[i].dma_cpu,
                                 g_acr[i].dma_handle);
        }
        if (g_acr[i].data) {
            lx_kfree(g_acr[i].data);
        }
        g_acr[i].data = NULL;
        g_acr[i].dma_cpu = NULL;
        g_acr[i].payload_len = 0;
        g_acr[i].valid = 0;
    }
    g_acr_loaded = 0;
}
