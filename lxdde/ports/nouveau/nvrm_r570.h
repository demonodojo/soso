/* Estructuras del ABI de GSP-RM r570, transcritas de upstream.
 *
 * Origen: `drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r570/nvrm/gsp.h` y
 * `.../rm/r535/nvrm/gsp.h` de Linux (SPDX MIT, (c) 2025 NVIDIA CORPORATION),
 * que a su vez son extractos de open-gpu-kernel-modules. Se copian tal cual —
 * son el contrato binario con el firmware, no código a reescribir— y por eso
 * mantienen el estilo y los nombres de allí en vez de los del resto del port.
 *
 * **Ninguna de estas structs lleva `#pragma pack`: la alineación es natural.**
 */
#ifndef NVRM_R570_H
#define NVRM_R570_H

#include <stdint.h>

typedef uint8_t  NvU8;
typedef uint16_t NvU16;
typedef uint32_t NvU32;
typedef uint64_t NvU64;
typedef NvU8     NvBool;
typedef NvU32    NV_STATUS;
typedef NvU32    NvHandle;

typedef struct
{
    NvU16               deviceID;           // deviceID
    NvU16               vendorID;           // vendorID
    NvU16               subdeviceID;        // subsystem deviceID
    NvU16               subvendorID;        // subsystem vendorID
    NvU8                revisionID;         // revision ID
} BUSINFO;

#define NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS             (16U)

typedef struct DOD_METHOD_DATA
{
    NV_STATUS status;
    NvU32     acpiIdListLen;
    NvU32     acpiIdList[NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS];
} DOD_METHOD_DATA;

typedef struct JT_METHOD_DATA
{
    NV_STATUS status;
    NvU32     jtCaps;
    NvU16     jtRevId;
    NvBool    bSBIOSCaps;
} JT_METHOD_DATA;

typedef struct MUX_METHOD_DATA_ELEMENT
{
    NvU32       acpiId;
    NvU32       mode;
    NV_STATUS   status;
} MUX_METHOD_DATA_ELEMENT;

#define NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS             (16U)

typedef struct MUX_METHOD_DATA
{
    NvU32                       tableLen;
    MUX_METHOD_DATA_ELEMENT     acpiIdMuxModeTable[NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS];
    MUX_METHOD_DATA_ELEMENT     acpiIdMuxPartTable[NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS];
    MUX_METHOD_DATA_ELEMENT     acpiIdMuxStateTable[NV0073_CTRL_SYSTEM_ACPI_ID_MAP_MAX_DISPLAYS];    
} MUX_METHOD_DATA;

typedef struct CAPS_METHOD_DATA
{
    NV_STATUS status;
    NvU32     optimusCaps;
} CAPS_METHOD_DATA;

typedef struct ACPI_METHOD_DATA
{
    NvBool                                               bValid;
    DOD_METHOD_DATA                                      dodMethodData;
    JT_METHOD_DATA                                       jtMethodData;
    MUX_METHOD_DATA                                      muxMethodData;
    CAPS_METHOD_DATA                                     capsMethodData;
} ACPI_METHOD_DATA;

typedef struct GSP_VF_INFO
{
    NvU32  totalVFs;
    NvU32  firstVFOffset;
    NvU64  FirstVFBar0Address;
    NvU64  FirstVFBar1Address;
    NvU64  FirstVFBar2Address;
    NvBool b64bitBar0;
    NvBool b64bitBar1;
    NvBool b64bitBar2;
} GSP_VF_INFO;

typedef struct
{
    // Link capabilities
    NvU32 linkCap;
} GSP_PCIE_CONFIG_REG;

typedef struct GspSystemInfo
{
    NvU64 gpuPhysAddr;
    NvU64 gpuPhysFbAddr;
    NvU64 gpuPhysInstAddr;
    NvU64 gpuPhysIoAddr;
    NvU64 nvDomainBusDeviceFunc;
    NvU64 simAccessBufPhysAddr;
    NvU64 notifyOpSharedSurfacePhysAddr;
    NvU64 pcieAtomicsOpMask;
    NvU64 consoleMemSize;
    NvU64 maxUserVa;
    NvU32 pciConfigMirrorBase;
    NvU32 pciConfigMirrorSize;
    NvU32 PCIDeviceID;
    NvU32 PCISubDeviceID;
    NvU32 PCIRevisionID;
    NvU32 pcieAtomicsCplDeviceCapMask;
    NvU8 oorArch;
    NvU64 clPdbProperties;
    NvU32 Chipset;
    NvBool bGpuBehindBridge;
    NvBool bFlrSupported;
    NvBool b64bBar0Supported;
    NvBool bMnocAvailable;
    NvU32  chipsetL1ssEnable;
    NvBool bUpstreamL0sUnsupported;
    NvBool bUpstreamL1Unsupported;
    NvBool bUpstreamL1PorSupported;
    NvBool bUpstreamL1PorMobileOnly;
    NvBool bSystemHasMux;
    NvU8   upstreamAddressValid;
    BUSINFO FHBBusInfo;
    BUSINFO chipsetIDInfo;
    ACPI_METHOD_DATA acpiMethodData;
    NvU32 hypervisorType;
    NvBool bIsPassthru;
    NvU64 sysTimerOffsetNs;
    GSP_VF_INFO gspVFInfo;
    NvBool bIsPrimary;
    NvBool isGridBuild;
    GSP_PCIE_CONFIG_REG pcieConfigReg;
    NvU32 gridBuildCsp;
    NvBool bPreserveVideoMemoryAllocations;
    NvBool bTdrEventSupported;
    NvBool bFeatureStretchVblankCapable;
    NvBool bEnableDynamicGranularityPageArrays;
    NvBool bClockBoostSupported;
    NvBool bRouteDispIntrsToCPU;
    NvU64  hostPageSize;
} GspSystemInfo;

/* Registry de GSP-RM (`rm/r535/nvrm/gsp.h`). El campo `data` es el valor si el
 * tipo es DWORD, o un offset dentro de la tabla si es BINARY/STRING. */
typedef struct PACKED_REGISTRY_ENTRY
{
    NvU32                   nameOffset;
    NvU8                    type;
    NvU32                   data;
    NvU32                   length;
} PACKED_REGISTRY_ENTRY;

typedef struct PACKED_REGISTRY_TABLE
{
    NvU32                   size;
    NvU32                   numEntries;
    /* entries[] y luego las cadenas de los nombres */
} PACKED_REGISTRY_TABLE;

#define REGISTRY_TABLE_ENTRY_TYPE_DWORD 1u

/* ---- Objetos de RM (G4c) ---------------------------------------------------
 *
 * GSP-RM expone un modelo de objetos: se pide un cliente, y colgando de él un
 * device y un subdevice. Todo va envuelto en estas dos cabeceras, que viajan
 * dentro del payload de un RPC (`rm/r535/nvrm/{alloc,ctrl}.h`).
 *
 * **No existen bajo r570/nvrm**: son compartidas con r535, sin divergencia de
 * versión — al contrario que `NV0000_ALLOC_PARAMETERS`, que en r570 añade
 * `pOsPidInfo` al final y en r535 no lo lleva.
 *
 * Cuidado con los DOS niveles de estado: el `rpc_result` de la cabecera RPC dice
 * si el transporte fue bien, y el `status` de aquí dentro es lo que responde RM.
 * Un RPC puede llegar impecable y traer un NV_STATUS de error dentro. */
typedef struct rpc_gsp_rm_alloc
{
    NvU32                   hClient;
    NvU32                   hParent;
    NvU32                   hObject;
    NvU32                   hClass;
    NvU32                   status;
    NvU32                   paramsSize;
    NvU32                   flags;
    NvU8                    reserved[4];
    /* params[] detrás */
} rpc_gsp_rm_alloc;

typedef struct rpc_gsp_rm_control
{
    NvU32                   hClient;
    NvU32                   hObject;
    NvU32                   cmd;
    NvU32                   status;
    NvU32                   paramsSize;
    NvU32                   flags;
    /* params[] detrás */
} rpc_gsp_rm_control;

/* Números de función de `rm/r570/nvrm/rpcfn.h`.
 *
 * `FREE` valía 27 aquí y es **10**. El 27 es `DMA_FILL_PTE_MEM`: liberar un
 * objeto le pedía a RM que rellenase PTEs interpretando los cuatro handles de
 * `NVOS00_PARAMETERS` como descriptor. Nunca llegó a dispararse porque el único
 * `gsp_rm_free` del árbol está en la ruta de error de `gsp_rm_init` y esa rama
 * no se tomó en el HW (2026-07-25), pero el apagado de G4 lo llama siempre.
 * Los otros tres números están confirmados contra hardware: RM_ALLOC y
 * RM_CONTROL respondieron `ok`, y GET_GSP_STATIC_INFO devolvió su propio fn. */
#define NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL  76u
#define NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC   103u
#define NV_VGPU_MSG_FUNCTION_FREE            10u
#define NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER 47u

/* `rpc_unloading_guest_driver_v1F_07` (`rm/r535/nvrm/gsp.h`; r570 la hereda).
 * El aviso de "me voy" que `r535_gsp_fini` manda antes de soltar la tarjeta.
 * `NvBool` es `NvU8`, así que la alineación natural mete 2 B de relleno antes
 * de `newLevel` y el struct sale a 8 B. */
typedef struct rpc_unloading_guest_driver_v1F_07
{
    NvBool                  bInPMTransition;
    NvBool                  bGc6Entering;
    NvU32                   newLevel;
} rpc_unloading_guest_driver_v1F_07;

/* `NV2080_CTRL_GPU_SET_POWER_STATE_GPU_LEVEL_0` — el nivel que pide un unload
 * que no es suspensión (upstream usa el 3 para la rama `suspend`). */
#define NV2080_CTRL_GPU_SET_POWER_STATE_GPU_LEVEL_0 0u

/* Handles, de `rm/handles.h`. RM no los inventa: los elegimos nosotros. */
#define NVKM_RM_CLIENT(id)  (0xc1d00000u | (id))
#define NVKM_RM_DEVICE       0xde1d0000u
#define NVKM_RM_SUBDEVICE    0x5d1d0000u
#define NVKM_RM_VASPACE      0x90f10000u
#define NVKM_RM_CHAN(chid)  (0xf1f00000u | (chid))

/* Clases y sus parámetros de reserva. */
#define NV01_ROOT          0x0u
#define NV01_DEVICE_0      0x80u
#define NV20_SUBDEVICE_0   0x2080u

#define NV_PROC_NAME_MAX_LENGTH 100u

/* r570 (`rm/r570/nvrm/client.h`) — el `pOsPidInfo` final es lo que la separa de
 * la versión r535. La alineación natural ya mete los 4 B de relleno tras
 * `processName`, así que sale a 120 B sin atributos. */
typedef struct NV0000_ALLOC_PARAMETERS
{
    NvU32                   hClient;
    NvU32                   processID;
    char                    processName[NV_PROC_NAME_MAX_LENGTH];
    NvU64                   pOsPidInfo;
} NV0000_ALLOC_PARAMETERS;

typedef struct NV0080_ALLOC_PARAMETERS
{
    NvU32                   deviceId;
    NvU32                   hClientShare;
    NvU32                   hTargetClient;
    NvU32                   hTargetDevice;
    NvU32                   flags;
    NvU64                   vaSpaceSize;
    NvU64                   vaStartInternal;
    NvU64                   vaLimitInternal;
    NvU32                   vaMode;
} NV0080_ALLOC_PARAMETERS;

typedef struct NV2080_ALLOC_PARAMETERS
{
    NvU32                   subDeviceId;
} NV2080_ALLOC_PARAMETERS;

/* ---- GspStaticConfigInfo (G4d) ---------------------------------------------
 *
 * Lo que RM cuenta de la GPU en cuanto arranca, por RPC (no es un control):
 * `GET_GSP_STATIC_INFO`. Tres cosas nos interesan de aquí:
 *
 *  - `fbRegionInfoParams`: las regiones de VRAM y cuáles son utilizables. Es la
 *    única forma honesta de saber el techo, porque en la ruta FMC el driver NO
 *    calcula los offsets del WPR — los pone el FMC y no los devuelve.
 *  - `hInternalClient/Device/Subdevice`: RM regala un juego de objetos ya hechos,
 *    aparte de los que pedimos en G4c.
 *  - `bar1PdeBase`/`bar2PdeBase`: RM **ya construyó** las tablas de páginas de
 *    BAR1/BAR2.
 *
 * Transcrito de `rm/r570/nvrm/gsp.h`. Todo lo anidado va aquí porque los offsets
 * de los campos que sí usamos dependen del prefijo entero: no hay atajo. */
#define NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MAX_ENTRIES 16u
#define NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MEM_TYPES   17u
#define NV0080_CTRL_GR_CAPS_TBL_SIZE                      23
#define NV2080_GPU_MAX_GID_LENGTH                         0x100u
#define NV2080_GPU_MAX_NAME_STRING_LENGTH                 0x40u
#define MAX_GROUP_COUNT                                   2
/* `RM_ENGINE_TYPE_LAST` de `rm/r570/nvrm/engine.h` — de él sale el tamaño de
 * `engineCaps[]`, así que si cambia, el struct entero se desplaza. */
#define RM_ENGINE_TYPE_LAST                               0x54u
#define NVGPU_ENGINE_CAPS_MASK_BITS                       32
#define NVGPU_ENGINE_CAPS_MASK_ARRAY_MAX \
    (((RM_ENGINE_TYPE_LAST - 1) / NVGPU_ENGINE_CAPS_MASK_BITS) + 1)

typedef NvBool
NV2080_CTRL_CMD_FB_GET_FB_REGION_SURFACE_MEM_TYPE_FLAG[NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MEM_TYPES];

typedef struct NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO
{
    NvU64                   base;
    NvU64                   limit;
    NvU64                   reserved;
    NvU32                   performance;
    NvBool                  supportCompressed;
    NvBool                  supportISO;
    NvBool                  bProtected;
    NV2080_CTRL_CMD_FB_GET_FB_REGION_SURFACE_MEM_TYPE_FLAG blackList;
} NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO;

typedef struct NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_PARAMS
{
    NvU32                   numFBRegions;
    NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO
        fbRegion[NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MAX_ENTRIES];
} NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_PARAMS;

typedef struct NV2080_CTRL_GPU_GET_GID_INFO_PARAMS
{
    NvU32                   index;
    NvU32                   flags;
    NvU32                   length;
    NvU8                    data[NV2080_GPU_MAX_GID_LENGTH];
} NV2080_CTRL_GPU_GET_GID_INFO_PARAMS;

typedef struct NV2080_CTRL_BIOS_GET_SKU_INFO_PARAMS
{
    NvU32                   BoardID;
    char                    chipSKU[9];
    char                    chipSKUMod[5];
    NvU32                   skuConfigVersion;
    char                    project[5];
    char                    projectSKU[5];
    char                    CDP[6];
    char                    projectSKUMod[2];
    NvU32                   businessCycle;
} NV2080_CTRL_BIOS_GET_SKU_INFO_PARAMS;

typedef struct NV0080_CTRL_GPU_GET_SRIOV_CAPS_PARAMS
{
    NvU32                   totalVFs;
    NvU32                   firstVfOffset;
    NvU32                   vfFeatureMask;
    NvU64                   FirstVFBar0Address;
    NvU64                   FirstVFBar1Address;
    NvU64                   FirstVFBar2Address;
    NvU64                   bar0Size;
    NvU64                   bar1Size;
    NvU64                   bar2Size;
    NvBool                  b64bitBar0;
    NvBool                  b64bitBar1;
    NvBool                  b64bitBar2;
    NvBool                  bSriovEnabled;
    NvBool                  bSriovHeavyEnabled;
    NvBool                  bEmulateVFBar0TlbInvalidationRegister;
    NvBool                  bClientRmAllocatedCtxBuffer;
    NvBool                  bNonPowerOf2ChannelCountSupported;
    NvBool                  bVfResizableBAR1Supported;
} NV0080_CTRL_GPU_GET_SRIOV_CAPS_PARAMS;

typedef struct VIRTUAL_DISPLAY_GET_NUM_HEADS_PARAMS
{
    NvU32                   numHeads;
    NvU32                   maxNumHeads;
} VIRTUAL_DISPLAY_GET_NUM_HEADS_PARAMS;

typedef struct VIRTUAL_DISPLAY_GET_MAX_RESOLUTION_PARAMS
{
    NvU32                   headIndex;
    NvU32                   maxHResolution;
    NvU32                   maxVResolution;
} VIRTUAL_DISPLAY_GET_MAX_RESOLUTION_PARAMS;

typedef struct EcidManufacturingInfo
{
    NvU32                   ecidLow;
    NvU32                   ecidHigh;
    NvU32                   ecidExtended;
} EcidManufacturingInfo;

typedef struct FW_WPR_LAYOUT_OFFSET
{
    NvU64                   nonWprHeapOffset;
    NvU64                   frtsOffset;
} FW_WPR_LAYOUT_OFFSET;

typedef struct GspStaticConfigInfo
{
    NvU8                    grCapsBits[NV0080_CTRL_GR_CAPS_TBL_SIZE];
    NV2080_CTRL_GPU_GET_GID_INFO_PARAMS          gidInfo;
    NV2080_CTRL_BIOS_GET_SKU_INFO_PARAMS         SKUInfo;
    NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_PARAMS fbRegionInfoParams;

    NV0080_CTRL_GPU_GET_SRIOV_CAPS_PARAMS        sriovCaps;
    NvU32                   sriovMaxGfid;

    NvU32                   engineCaps[NVGPU_ENGINE_CAPS_MASK_ARRAY_MAX];

    NvBool                  poisonFuseEnabled;

    NvU64                   fb_length;
    NvU64                   fbio_mask;
    NvU32                   fb_bus_width;
    NvU32                   fb_ram_type;
    NvU64                   fbp_mask;
    NvU32                   l2_cache_size;

    NvU8                    gpuNameString[NV2080_GPU_MAX_NAME_STRING_LENGTH];
    NvU8                    gpuShortNameString[NV2080_GPU_MAX_NAME_STRING_LENGTH];
    NvU16                   gpuNameString_Unicode[NV2080_GPU_MAX_NAME_STRING_LENGTH];
    NvBool                  bGpuInternalSku;
    NvBool                  bIsQuadroGeneric;
    NvBool                  bIsQuadroAd;
    NvBool                  bIsNvidiaNvs;
    NvBool                  bIsVgx;
    NvBool                  bGeforceSmb;
    NvBool                  bIsTitan;
    NvBool                  bIsTesla;
    NvBool                  bIsMobile;
    NvBool                  bIsGc6Rtd3Allowed;
    NvBool                  bIsGc8Rtd3Allowed;
    NvBool                  bIsGcOffRtd3Allowed;
    NvBool                  bIsGcoffLegacyAllowed;
    NvBool                  bIsMigSupported;

    NvU16                   RTD3GC6TotalBoardPower;
    NvU16                   RTD3GC6PerstDelay;

    NvU64                   bar1PdeBase;
    NvU64                   bar2PdeBase;

    NvBool                  bVbiosValid;
    NvU32                   vbiosSubVendor;
    NvU32                   vbiosSubDevice;

    NvBool                  bPageRetirementSupported;
    NvBool                  bSplitVasBetweenServerClientRm;
    NvBool                  bClRootportNeedsNosnoopWAR;

    VIRTUAL_DISPLAY_GET_NUM_HEADS_PARAMS       displaylessMaxHeads;
    VIRTUAL_DISPLAY_GET_MAX_RESOLUTION_PARAMS  displaylessMaxResolution;
    NvU64                   displaylessMaxPixels;

    NvHandle                hInternalClient;
    NvHandle                hInternalDevice;
    NvHandle                hInternalSubdevice;

    NvBool                  bSelfHostedMode;
    NvBool                  bAtsSupported;

    NvBool                  bIsGpuUefi;
    NvBool                  bIsEfiInit;

    EcidManufacturingInfo   ecidInfo[MAX_GROUP_COUNT];

    FW_WPR_LAYOUT_OFFSET    fwWprLayoutOffset;
} GspStaticConfigInfo;

#define NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO 65u

/* ---- Espacio de direcciones (G4d 2/2) --------------------------------------
 *
 * Transcrito de `rm/r535/nvrm/vmm.h` (r570 lo hereda sin tocar) y usado por
 * `r535_mmu_vaspace_new`.
 *
 * El modelo importa más que los campos: con `IS_EXTERNALLY_OWNED` **las tablas
 * de páginas son nuestras**. RM no las construye ni las toca; solo se le dice
 * dónde está la raíz con `SET_PAGE_DIRECTORY`. Es lo que hace nouveau para el
 * VMM que promociona (`r535_mmu_promote_vmm` pasa `external=true`), y para
 * nosotros es además el camino más corto: la alternativa —dejar el espacio en
 * manos de RM— exige igualmente construir un directorio y encima copiarle los
 * PDE de su región reservada.  */
#define FERMI_VASPACE_A 0x90f1u

typedef struct NV_VASPACE_ALLOCATION_PARAMETERS
{
    NvU32                   index;
    NvU32                   flags;
    NvU64                   vaSize;
    NvU64                   vaStartInternal;
    NvU64                   vaLimitInternal;
    NvU32                   bigPageSize;
    NvU64                   vaBase;
} NV_VASPACE_ALLOCATION_PARAMETERS;

#define NV_VASPACE_ALLOCATION_INDEX_GPU_NEW               0x00u
#define NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED   (1u << 3)

/* Controles sobre el device (`NV01_DEVICE_0`), no sobre el vaspace. */
#define NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY   0x801813u
#define NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY 0x801814u

typedef struct NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS
{
    NvU64                   physAddress;
    NvU32                   numEntries;
    NvU32                   flags;
    NvU32                   hVASpace;
    NvU32                   chId;
    NvU32                   subDeviceId;    /* ID+1; 0 = broadcast */
    NvU32                   pasid;
} NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS;

/* `FLAGS_APERTURE` es 1:0. Upstream pone VIDMEM porque su directorio vive en
 * VRAM (lo reserva instmem); el nuestro vive en sysmem coherente, que es lo
 * único que la CPU puede escribir hoy aquí — sin BAR1 no hay ventana a la VRAM. */
#define NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_VIDMEM       0u
#define NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_SYSMEM_COH   1u
#define NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_SYSMEM_NONCOH 2u

typedef struct NV0080_CTRL_DMA_UNSET_PAGE_DIRECTORY_PARAMS
{
    NvU32                   hVASpace;
    NvU32                   subDeviceId;
} NV0080_CTRL_DMA_UNSET_PAGE_DIRECTORY_PARAMS;

/* Si un tamaño baila, RM lee los campos desplazados y responde cualquier cosa. */
typedef char rm_alloc_hdr_size_check[sizeof(rpc_gsp_rm_alloc) == 32 ? 1 : -1];
typedef char rm_ctrl_hdr_size_check[sizeof(rpc_gsp_rm_control) == 24 ? 1 : -1];
typedef char nv0000_size_check[sizeof(NV0000_ALLOC_PARAMETERS) == 120 ? 1 : -1];
typedef char nv0080_size_check[sizeof(NV0080_ALLOC_PARAMETERS) == 56 ? 1 : -1];
typedef char nv2080_size_check[sizeof(NV2080_ALLOC_PARAMETERS) == 4 ? 1 : -1];
/* 1656 B con alineación natural. No hay verdad externa contra la que comparar
 * este número —upstream no lo asserta—, así que el assert solo detecta que
 * ALGUIEN cambie el struct: la validación de verdad es el contraste de
 * `fb_length` contra la VRAM que ya conocemos por registro. */
typedef char gsp_static_info_size_check[sizeof(GspStaticConfigInfo) == 1656 ? 1 : -1];
typedef char fb_region_size_check[
    sizeof(NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO) == 48 ? 1 : -1];
typedef char rpc_unload_size_check[
    sizeof(rpc_unloading_guest_driver_v1F_07) == 8 ? 1 : -1];
/* 48 y 32 con alineación natural: los `NV_ALIGN_BYTES(8)` de upstream no añaden
 * nada sobre x86-64, pero sí explican el hueco de 4 B tras `index`/`flags` y el
 * de 4 B tras `bigPageSize`. Si estos números bailan, el `index` que RM lee no
 * es el que mandamos y el vaspace sale con la geometría de otra cosa. */
typedef char nv_vaspace_size_check[
    sizeof(NV_VASPACE_ALLOCATION_PARAMETERS) == 48 ? 1 : -1];
typedef char nv0080_set_pd_size_check[
    sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS) == 32 ? 1 : -1];

#endif
