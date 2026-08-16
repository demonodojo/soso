#include "gsp_chip.h"
#include "nvrm_r570.h"

static uint32_t g_chip_boot0;
static uint16_t g_chip_device_id;

int gsp_nv_boot0_valid(uint32_t boot0)
{
    /* 0xffffffff = GPU fuera del bus / D3cold; 0 = BAR0 sin asignar. */
    return boot0 != 0xffffffffu && boot0 != 0u;
}

void gsp_nv_family_set(uint32_t boot0, uint16_t device_id)
{
    g_chip_boot0 = boot0;
    g_chip_device_id = device_id;
}

uint32_t gsp_nv_family_boot0(void)
{
    return g_chip_boot0;
}

uint16_t gsp_nv_family_device_id(void)
{
    return g_chip_device_id;
}

enum nv_family gsp_nv_family_of(uint32_t boot0, uint16_t device_id)
{
    if (gsp_nv_boot0_valid(boot0)) {
        unsigned arch = (boot0 >> 20) & 0x1ffu;

        if (arch >= 0x1a0u)
            return NV_FAM_BLACKWELL;
        if (arch >= 0x190u)
            return NV_FAM_ADA;
        if (arch >= 0x170u)
            return NV_FAM_AMPERE;
    }
    if (device_id == GB205_DEVICE_ID)
        return NV_FAM_BLACKWELL;
    if ((device_id & 0xff00u) >= 0x2200u && (device_id & 0xff00u) <= 0x25ffu)
        return NV_FAM_AMPERE;
    return NV_FAM_UNKNOWN;
}

enum nv_family gsp_nv_family_current(void)
{
    return gsp_nv_family_of(g_chip_boot0, g_chip_device_id);
}

const char *gsp_nv_ampere_chip_name(uint16_t device_id)
{
    if (device_id == GA107_DEVICE_ID)
        return "ga107";
    return "ga102";
}

const char *gsp_nv_chip_name(uint16_t device_id, enum nv_family fam)
{
    switch (fam) {
    case NV_FAM_BLACKWELL:
        return "gb205";
    case NV_FAM_AMPERE:
        return gsp_nv_ampere_chip_name(device_id);
    case NV_FAM_ADA:
        return "ad10x";
    default:
        return "unknown";
    }
}

const char *gsp_nv_family_name(enum nv_family f)
{
    enum nv_family cur = gsp_nv_family_current();

    switch (f) {
    case NV_FAM_AMPERE:
        return gsp_nv_ampere_chip_name(g_chip_device_id);
    case NV_FAM_ADA:
        return "Ada (ad10x)";
    case NV_FAM_BLACKWELL:
        return "Blackwell (gb20x)";
    default:
        if (cur != NV_FAM_UNKNOWN)
            return gsp_nv_chip_name(g_chip_device_id, cur);
        return "desconocida";
    }
}

uint32_t gsp_chan_doorbell_kick(enum nv_family fam, uint32_t rpc_token)
{
    uint32_t runlist = (rpc_token >> 16) & 0x7fu;
    uint32_t chid = rpc_token & NV_VF_DOORBELL_VECTOR_MASK;
    uint32_t kick = (runlist << 16) | chid;

    if (fam == NV_FAM_BLACKWELL)
        kick |= NV_VF_DOORBELL_RUNLIST_DOORBELL_ENABLE;
    return kick;
}
