/* N3 — puente hacia nvkm disp; hoy enlaza VBIOS nvkm + probe PROM. */
#include "nvdisp_engine.h"
#include "nvdisp_head.h"
#include "nvdisp_probe.h"
#include "gsp_rom.h"
#include "gsp_mmio.h"
#include "lx_emul.h"

int nvdisp_engine_nvkm_bios_linked(void)
{
    /* Ola 0 N3: image/shadow/bit/dcb en source.list → símbolos nvkm reales. */
    return 1;
}

int nvdisp_engine_init(void)
{
    if (!gsp_mmio_alive()) {
        lx_printk("NVDisplay: engine omitido (sin BAR0)\n");
        return -1;
    }
    gsp_rom_init();
    if (nvdisp_probe() != 0) {
        lx_printk("NVDisplay: engine sin VBIOS/DCB\n");
        return -1;
    }
    if (nvdisp_engine_nvkm_bios_linked()) {
        lx_printk("NVDisplay: engine N3 (nvkm bios en liblxdde)\n");
    } else {
        lx_printk("NVDisplay: engine N3 (solo probe PROM)\n");
    }
    return 0;
}
