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

static const struct gsp_fw_slot g_paths[GSP_FW_COUNT] = {
    { "nvidia/gb205/gsp/bootloader-570.144.bin", GSP_FW_BOOTLOADER, 0, 1 },
    { "nvidia/gb205/gsp/fmc-570.144.bin", GSP_FW_FMC, 1, 0 },
    { "nvidia/gb205/gsp/gsp-570.144.bin", GSP_FW_UCODE, 1, 0 },
    { "nvidia/ga102/gsp/gsp-570.144.bin", GSP_FW_GA102_UCODE, 1, 0 },
};

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

int gsp_fw_load_all(void)
{
    unsigned i;
    int ok = 0;

    if (g_loaded) {
        return 0;
    }
    for (i = 0; i < GSP_FW_COUNT; i++) {
        g_blobs[i].path = g_paths[i].path;
        g_blobs[i].data = NULL;
        g_blobs[i].len = 0;
        g_blobs[i].gem_handle = 0;
        g_blobs[i].valid = 0;
    }
    for (i = 0; i < GSP_FW_COUNT; i++) {
        if (load_one(&g_paths[i], &g_blobs[i]) == 0) {
            ok++;
            lx_printk("nouveau-lx: fw %s (%lu bytes)\n", g_blobs[i].path, g_blobs[i].len);
        }
    }
    if (ok < 3) {
        lx_printk("nouveau-lx: GSP gb205 incompleto (%d blobs)\n", ok);
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
    int staged = 0;

    for (i = 0; i < GSP_FW_COUNT; i++) {
        struct gsp_fw_blob *b = &g_blobs[i];
        void *va;
        unsigned long off;
        if (!b->valid || !b->data) {
            continue;
        }
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
    return staged >= 3 ? 0 : -1;
}

void gsp_fw_release_all(void)
{
    unsigned i;
    for (i = 0; i < GSP_FW_COUNT; i++) {
        if (g_blobs[i].data) {
            lx_kfree(g_blobs[i].data);
        }
        g_blobs[i].data = NULL;
        g_blobs[i].len = 0;
        g_blobs[i].gem_handle = 0;
        g_blobs[i].valid = 0;
    }
    g_loaded = 0;
}
