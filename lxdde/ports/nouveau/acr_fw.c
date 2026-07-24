/* G3 ola 2: firmware ACR ga102 (AHESASC + ASB) en heap lx + DMA. */
#include "acr_fw.h"
#include "lx_emul.h"

static struct acr_fw_blob g_acr[ACR_FW_COUNT];
static int g_acr_loaded;

static const char *const g_acr_paths[ACR_FW_COUNT] = {
    "nvidia/ga102/acr/ucode_ahesasc.bin",
    "nvidia/ga102/acr/ucode_asb.bin",
};

static int load_one(enum acr_fw_kind kind, const char *path)
{
    const unsigned char *tmp = NULL;
    unsigned long tmp_len = 0;
    unsigned char *copy;
    uint64_t dma_handle = 0;
    unsigned char *dma_cpu;
    struct acr_fw_blob *b = &g_acr[kind];

    if (lx_request_firmware(path, &tmp, &tmp_len) != 0 || !tmp || tmp_len < 64) {
        return -1;
    }
    copy = lx_kmalloc(tmp_len, GFP_KERNEL);
    if (!copy) {
        lx_release_firmware(tmp);
        return -1;
    }
    {
        unsigned long i;
        for (i = 0; i < tmp_len; i++) {
            copy[i] = tmp[i];
        }
    }
    lx_release_firmware(tmp);

    dma_cpu = lx_dma_alloc_coherent(NULL, tmp_len, &dma_handle, GFP_KERNEL);
    if (!dma_cpu) {
        lx_kfree(copy);
        return -1;
    }
    {
        unsigned long i;
        for (i = 0; i < tmp_len; i++) {
            dma_cpu[i] = copy[i];
        }
    }

    b->path = path;
    b->data = copy;
    b->len = tmp_len;
    b->dma_cpu = dma_cpu;
    b->dma_handle = (unsigned)dma_handle;
    b->valid = 1;
    lx_printk("nouveau-lx: acr fw %s (%lu bytes, dma=0x%x)\n", path, tmp_len, b->dma_handle);
    return 0;
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
        if (load_one((enum acr_fw_kind)i, g_acr_paths[i]) == 0) {
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
            lx_dma_free_coherent(NULL, g_acr[i].len, g_acr[i].dma_cpu, g_acr[i].dma_handle);
        }
        if (g_acr[i].data) {
            lx_kfree(g_acr[i].data);
        }
        g_acr[i].data = NULL;
        g_acr[i].dma_cpu = NULL;
        g_acr[i].valid = 0;
    }
    g_acr_loaded = 0;
}
