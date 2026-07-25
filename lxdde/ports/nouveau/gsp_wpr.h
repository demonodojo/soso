/* G3b paso 4: `GspFwWprMeta` — el descriptor que el GSP-FMC lee para montar la
 * región protegida (WPR2) en VRAM y encontrar el firmware en sysmem.
 *
 * En la ruta FMC (GH100+, incluido GB20x) el driver **no** calcula direcciones de
 * WPR: eso lo hace el FMC (`offset_set_by_acr`). Solo rellena tamaños y punteros a
 * sysmem. Referencia: `gh100_gsp_wpr_meta_init` en `nvkm/subdev/gsp/gh100.c` y la
 * estructura de `nvkm/subdev/gsp/rm/r570/nvrm/gsp.h` (upstream; el árbol 6.6
 * pinneado en lxdde/linux no tiene nada de esto).
 */
#ifndef GSP_WPR_H
#define GSP_WPR_H

#include "gsp_rm.h"

/* El bootloader (`bootloader-*.bin`) es una imagen RISC-V con cabecera NVIDIA;
 * al FMC hay que darle su dirección física y los offsets internos. */
struct gsp_boot_fw {
    unsigned char *va;
    uint64_t phys;
    unsigned long size;
    uint32_t code_offset;
    uint32_t data_offset;
    uint32_t manifest_offset;
    uint32_t app_version;
};

/* Exactamente 256 bytes: lo comprueba un assert de compilación en gsp_wpr.c.
 * Los dos huecos que upstream declara como `union` están puestos con la variante
 * de arranque inicial, que es la única que usamos (la otra ocupa lo mismo). */
struct gsp_wpr_meta {
    uint64_t magic;                     /* 0x00 */
    uint64_t revision;                  /* 0x08 */

    uint64_t sysmemAddrOfRadix3Elf;     /* 0x10 */
    uint64_t sizeOfRadix3Elf;           /* 0x18 */
    uint64_t sysmemAddrOfBootloader;    /* 0x20 */
    uint64_t sizeOfBootloader;          /* 0x28 */
    uint64_t bootloaderCodeOffset;      /* 0x30 */
    uint64_t bootloaderDataOffset;      /* 0x38 */
    uint64_t bootloaderManifestOffset;  /* 0x40 */

    /* union: en resume son gspFwHeapFreeListWprOffset/unused. */
    uint64_t sysmemAddrOfSignature;     /* 0x48 */
    uint64_t sizeOfSignature;           /* 0x50 */

    uint64_t gspFwRsvdStart;            /* 0x58 */
    uint64_t nonWprHeapOffset;          /* 0x60 */
    uint64_t nonWprHeapSize;            /* 0x68 */
    uint64_t gspFwWprStart;             /* 0x70 */
    uint64_t gspFwHeapOffset;           /* 0x78 */
    uint64_t gspFwHeapSize;             /* 0x80 */
    uint64_t gspFwOffset;               /* 0x88 */
    uint64_t bootBinOffset;             /* 0x90 */
    uint64_t frtsOffset;                /* 0x98 */
    uint64_t frtsSize;                  /* 0xa0 */
    uint64_t gspFwWprEnd;               /* 0xa8 */
    uint64_t fbSize;                    /* 0xb0 */
    uint64_t vgaWorkspaceOffset;        /* 0xb8 */
    uint64_t vgaWorkspaceSize;          /* 0xc0 */
    uint64_t bootCount;                 /* 0xc8 */

    /* union de 32 B: variante de arranque inicial (la otra es CrashCat). */
    uint64_t partitionRpcAddr;          /* 0xd0 */
    uint16_t partitionRpcRequestOffset; /* 0xd8 */
    uint16_t partitionRpcReplyOffset;   /* 0xda */
    uint32_t elfCodeOffset;             /* 0xdc */
    uint32_t elfDataOffset;             /* 0xe0 */
    uint32_t elfCodeSize;               /* 0xe4 */
    uint32_t elfDataSize;               /* 0xe8 */
    uint32_t lsUcodeVersion;            /* 0xec */

    uint8_t gspFwHeapVfPartitionCount;  /* 0xf0 */
    uint8_t flags;                      /* 0xf1 */
    uint8_t padding[2];                 /* 0xf2 */
    uint32_t pmuReservedSize;           /* 0xf4 */
    uint64_t verified;                  /* 0xf8 */
};

struct gsp_wpr {
    struct gsp_boot_fw boot;
    struct gsp_wpr_meta *meta;
    uint64_t meta_phys;
    uint64_t fb_bytes;      /* VRAM real, leída de 0x1183a4 */
    uint64_t heap_size;
    /* Lo que el COT reserva al final de la VRAM: heap fuera de WPR + la reserva
     * del PMU, alineado a 2 MiB (`rsvd_size` en `gh100_gsp_init`). */
    uint32_t rsvd_size;
    int ready;
};

/* VRAM en bytes según el hardware (`ga102_fb_vidmem_size`: 0x1183a4 en MiB).
 * 0 si el registro no responde. Requiere BAR0 mapeada. */
uint64_t gsp_wpr_vidmem_size(void);

/* Prepara el bootloader en memoria DMA y construye el WPR meta a partir de la
 * imagen GSP-RM ya preparada. Solo lee registros (la VRAM); no escribe ninguno.
 * Solo vale para la ruta FMC (GB20x/GH100): en Ampere el layout de WPR lo calcula
 * el driver entero y es otra función. */
int gsp_wpr_prepare(const struct gsp_rm_fw *rm, struct gsp_wpr *out);

void gsp_wpr_release(struct gsp_wpr *w);

#endif
