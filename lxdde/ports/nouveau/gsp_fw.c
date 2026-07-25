/* G3: carga y staging GEM de firmware GSP. */
#include "gsp_fw.h"
#include "lx_emul.h"

struct gsp_fw_slot {
    const char *path;
    enum gsp_fw_kind kind;
    int is_elf;
    int is_boot_nv;
};

static struct gsp_fw_blob g_blobs[GSP_FW_COUNT];
static int g_loaded;

/* Blackwell arranca por FMC: bootloader + fmc + ucode. */
static const struct gsp_fw_slot g_paths_blackwell[] = {
    { "nvidia/gb205/gsp/bootloader-570.144.bin", GSP_FW_BOOTLOADER, 0, 1 },
    { "nvidia/gb205/gsp/fmc-570.144.bin", GSP_FW_FMC, 1, 0 },
    { "nvidia/gb205/gsp/gsp-570.144.bin", GSP_FW_UCODE, 1, 0 },
};

/* Ampere no tiene fmc: el ACR va por booter_load/booter_unload. */
static const struct gsp_fw_slot g_paths_ampere[] = {
    { "nvidia/ga102/gsp/bootloader-570.144.bin", GSP_FW_BOOTLOADER, 0, 1 },
    { "nvidia/ga102/gsp/booter_load-570.144.bin", GSP_FW_BOOTER_LOAD, 0, 0 },
    { "nvidia/ga102/gsp/booter_unload-570.144.bin", GSP_FW_BOOTER_UNLOAD, 0, 0 },
    { "nvidia/ga102/gsp/gsp-570.144.bin", GSP_FW_UCODE, 1, 0 },
};

/* En este linux-firmware `gb205/gsp/gsp-*.bin` es un symlink al de ga102: cargar
 * los dos juegos duplicaba 60,6 MiB de heap para tener los mismos bytes. */
static const struct gsp_fw_slot *chip_paths(enum gsp_fw_chip chip, unsigned *count)
{
    if (chip == GSP_FW_CHIP_AMPERE) {
        *count = sizeof(g_paths_ampere) / sizeof(g_paths_ampere[0]);
        return g_paths_ampere;
    }
    *count = sizeof(g_paths_blackwell) / sizeof(g_paths_blackwell[0]);
    return g_paths_blackwell;
}

static const char *chip_name(enum gsp_fw_chip chip)
{
    return chip == GSP_FW_CHIP_AMPERE ? "ga102" : "gb205";
}

static int blob_valid(const unsigned char *data, unsigned long len, const struct gsp_fw_slot *slot)
{
    if (!data || len < 4) {
        return 0;
    }
    if (slot->is_elf) {
        return data[0] == 0x7f && data[1] == 'E' && data[2] == 'L' && data[3] == 'F';
    }
    if (slot->is_boot_nv) {
        return len >= 256 && data[0] == 0xde && data[1] == 0x10;
    }
    return len > 0;
}

static int load_one(const struct gsp_fw_slot *slot, struct gsp_fw_blob *out)
{
    const unsigned char *tmp = NULL;
    unsigned long tmp_len = 0;
    unsigned char *copy;

    if (lx_request_firmware(slot->path, &tmp, &tmp_len) != 0 || !tmp || tmp_len == 0) {
        return -1;
    }
    if (!blob_valid(tmp, tmp_len, slot)) {
        lx_release_firmware(tmp);
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
    out->path = slot->path;
    out->data = copy;
    out->len = tmp_len;
    out->gem_handle = 0;
    out->valid = 1;
    return 0;
}

int gsp_fw_load_all(enum gsp_fw_chip chip)
{
    const struct gsp_fw_slot *paths;
    unsigned count;
    unsigned i;
    unsigned ok = 0;

    if (g_loaded) {
        return 0;
    }
    paths = chip_paths(chip, &count);
    for (i = 0; i < GSP_FW_COUNT; i++) {
        g_blobs[i].path = NULL;
        g_blobs[i].data = NULL;
        g_blobs[i].len = 0;
        g_blobs[i].gem_handle = 0;
        g_blobs[i].valid = 0;
    }
    for (i = 0; i < count; i++) {
        struct gsp_fw_blob *b = &g_blobs[paths[i].kind];
        b->path = paths[i].path;
        if (load_one(&paths[i], b) == 0) {
            ok++;
            lx_printk("nouveau-lx: fw %s (%lu bytes)\n", b->path, b->len);
        }
    }
    if (ok != count) {
        lx_printk("nouveau-lx: GSP %s incompleto (%u/%u blobs)\n",
                  chip_name(chip), ok, count);
        gsp_fw_release_all();
        return -1;
    }
    g_loaded = 1;
    return 0;
}

const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind kind)
{
    if (!g_loaded || kind >= GSP_FW_COUNT) {
        return NULL;
    }
    if (!g_blobs[kind].valid) {
        return NULL;
    }
    return &g_blobs[kind];
}

int gsp_fw_stage_all(void)
{
    unsigned i;
    unsigned staged = 0;
    unsigned expect = 0;

    for (i = 0; i < GSP_FW_COUNT; i++) {
        struct gsp_fw_blob *b = &g_blobs[i];
        void *va;
        unsigned long off;
        if (!b->valid || !b->data) {
            continue;
        }
        /* El ucode NO se copia a un GEM: son 60,6 MiB que el GSP lee por DMA
         * desde la radix3 de `gsp_rm.c`, no desde un objeto gráfico. Duplicarlo
         * aquí solo servía para dejar el heap al borde. */
        if (i == GSP_FW_UCODE) {
            continue;
        }
        expect++;
        if (b->gem_handle != 0) {
            staged++;
            continue;
        }
        b->gem_handle = lx_drm_gem_create(b->len);
        if (b->gem_handle == 0) {
            return -1;
        }
        va = lx_drm_gem_vmap(b->gem_handle);
        if (!va) {
            return -1;
        }
        {
            unsigned char *dst = va;
            for (off = 0; off < b->len; off++) {
                dst[off] = b->data[off];
            }
        }
        staged++;
        lx_printk("nouveau-lx: staged GEM h=%u %s (%lu bytes)\n",
                  b->gem_handle, b->path, b->len);
    }
    return staged == expect && expect > 0 ? 0 : -1;
}

void gsp_fw_release_one(enum gsp_fw_kind kind)
{
    struct gsp_fw_blob *b;

    if (kind >= GSP_FW_COUNT) {
        return;
    }
    b = &g_blobs[kind];
    if (b->data) {
        lx_kfree(b->data);
    }
    b->data = NULL;
    b->len = 0;
    b->valid = 0;
}

void gsp_fw_release_all(void)
{
    unsigned i;
    for (i = 0; i < GSP_FW_COUNT; i++) {
        if (g_blobs[i].data) {
            lx_kfree(g_blobs[i].data);
        }
        g_blobs[i].path = NULL;
        g_blobs[i].data = NULL;
        g_blobs[i].len = 0;
        g_blobs[i].gem_handle = 0;
        g_blobs[i].valid = 0;
    }
    g_loaded = 0;
}
