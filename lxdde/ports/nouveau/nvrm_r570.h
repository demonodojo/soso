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

/* ---- Canal GPFIFO + CE (G4e) -----------------------------------------------
 *
 * Referencias: `class/clc56f.h`, `class/clc6b5.h`, `alloc/alloc_channel.h`
 * (open-gpu-kernel-modules). Ampere/Blackwell usan AMPERE_CHANNEL_GPFIFO_A
 * y AMPERE_DMA_COPY_A; GB205 comparte la ruta Ampere en RM 570.144. */
#define NV_MAX_SUBDEVICES 32u

#define AMPERE_CHANNEL_GPFIFO_A  0x0000c56fu
#define AMPERE_DMA_COPY_A          0x0000c6b5u

#define NVKM_RM_CE0                0xc6b50000u

/* `NV2080_ENGINE_TYPE_COPY(i)` de `ctrl/ctrl2080/ctrl2080internal.h`. */
#define NV2080_ENGINE_TYPE_COPY0   6u

/* `addressSpace` de NV_MEMORY_DESC_PARAMS es el enum `NV_ADDRESS_SPACE` de RM,
 * y NO el `FLAGS_APERTURE` de SET_PAGE_DIRECTORY que hay 50 líneas más arriba
 * (donde SYSMEM_COH sí es 1 y VIDMEM 0). Confundirlos costó el `4` que había
 * aquí como "SYSMEM_COHERENT": en `r535_chan_alloc` upstream pone 2 en los tres
 * descriptores que viven en VRAM (instance, USERD, ramfc) y 1 en el único que
 * vive en sysmem (mthdbuf), así que 1=sysmem y 2=VRAM. El 4 no es sysmem. */
#define NV_ADDRESS_SPACE_SYSMEM           1u
#define NV_ADDRESS_SPACE_FBMEM            2u
#define NV_CACHE_ATTR_DEFAULT             0u
#define NV_CACHE_ATTR_CACHED              1u

/* `NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS` (alloc_channel.h). El enum de tipo de
 * notificador es UNKNOWN=0, NONE=1, CTXDMA=2, MEMORY=3 — o sea que dejar
 * `internalFlags` a cero NO pide "sin notificador", pide UNKNOWN. */
#define NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_PRIVILEGE_USER   0u
#define NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_PRIVILEGE_ADMIN  1u
#define NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ERROR_NOTIFIER_TYPE_NONE      (1u << 2)
#define NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ECC_ERROR_NOTIFIER_TYPE_NONE  (1u << 4)

typedef struct NV_MEMORY_DESC_PARAMS {
    NvU64 base;
    NvU64 size;
    NvU32 addressSpace;
    NvU32 cacheAttrib;
} NV_MEMORY_DESC_PARAMS;

typedef struct Nvc56fControl {
    NvU32 Ignored00[0x010];
    NvU32 Put;
    NvU32 Get;
    NvU32 Reference;
    NvU32 PutHi;
    NvU32 Ignored01[0x002];
    NvU32 TopLevelGet;
    NvU32 TopLevelGetHi;
    NvU32 GetHi;
    NvU32 Ignored02[0x007];
    NvU32 Ignored03;
    NvU32 Ignored04[0x001];
    NvU32 GPGet;
    NvU32 GPPut;
    NvU32 Ignored05[0x5c];
} Nvc56fControl;

typedef struct NV_CHANNEL_ALLOC_PARAMS {
    NvHandle hObjectError;
    NvHandle hObjectBuffer;
    NvU64    gpFifoOffset;
    NvU32    gpFifoEntries;
    NvU32    flags;
    NvHandle hContextShare;
    NvHandle hVASpace;
    /* Aquí NO va ningún `hHandleVASpace`: no existe ni en r535 ni en r570
     * (`rm/r570/nvrm/fifo.h`). El que había metía 4 B de más y desplazaba TODO
     * lo que viene detrás —engineType, subDeviceId y los cuatro descriptores de
     * memoria—, así que RM los leía corridos y contestaba
     * NV_ERR_INVALID_PARAMETER (0x3b) al RM_ALLOC del canal (2026-07-27). El
     * hostcheck no lo veía porque releía los campos con esta misma definición:
     * un layout mal pero coherente consigo mismo es invisible desde dentro. De
     * ahí el assert de tamaño de abajo, que sí lo ancla contra upstream. */
    NvHandle hUserdMemory[NV_MAX_SUBDEVICES];
    NvU64    userdOffset[NV_MAX_SUBDEVICES];
    NvU32    engineType;
    NvU32    cid;
    NvU32    subDeviceId;
    NvHandle hObjectEccError;
    NV_MEMORY_DESC_PARAMS instanceMem;
    NV_MEMORY_DESC_PARAMS userdMem;
    NV_MEMORY_DESC_PARAMS ramfcMem;
    NV_MEMORY_DESC_PARAMS mthdbufMem;
    NvHandle hPhysChannelGroup;
    NvU32    internalFlags;
    NV_MEMORY_DESC_PARAMS errorNotifierMem;
    NV_MEMORY_DESC_PARAMS eccErrorNotifierMem;
    NvU32    ProcessID;
    NvU32    SubProcessID;
    NvU32    encryptIv[3];
    NvU32    decryptIv[3];
    NvU32    hmacNonce[8];
    NvU32    tpcConfigID;
} NV_CHANNEL_ALLOC_PARAMS;

/* Ancla contra upstream, no contra nosotros mismos. Con el `hHandleVASpace` de
 * más salían 664 y este assert es justo lo que faltaba para cazarlo. */
typedef char nv_channel_alloc_size_check[
    sizeof(NV_CHANNEL_ALLOC_PARAMS) == 656 ? 1 : -1];
typedef char nv_channel_alloc_userd_off_check[
    offsetof(NV_CHANNEL_ALLOC_PARAMS, hUserdMemory) == 32 ? 1 : -1];
typedef char nv_channel_alloc_instmem_off_check[
    offsetof(NV_CHANNEL_ALLOC_PARAMS, instanceMem) == 432 ? 1 : -1];

#define NVOS04_FLAGS_CHANNEL_TYPE_PHYSICAL  0x00000000u
#define NVOS04_FLAGS_CHANNEL_CLIENT_MAP_FIFO  (1u << 24)

/* Bloque de instancia del canal. Upstream (`r535_chan_alloc`) apunta
 * `instanceMem` al bloque entero y `ramfcMem` a sus primeros 0x200 B, los dos
 * en VRAM. */
#define GSP_CHAN_INST_SIZE   0x1000u
#define GSP_CHAN_RAMFC_SIZE  0x200u

/* Métodos CE (class/clc6b5.h) — copia lineal virtual. */
#define NVC6B5_SET_OBJECT                    0x00000000u
#define NVC6B5_OFFSET_IN_UPPER                 0x00000400u
#define NVC6B5_OFFSET_IN_LOWER                 0x00000404u
#define NVC6B5_OFFSET_OUT_UPPER                0x00000408u
#define NVC6B5_OFFSET_OUT_LOWER                0x0000040cu
#define NVC6B5_PITCH_IN                        0x00000410u
#define NVC6B5_PITCH_OUT                       0x00000414u
#define NVC6B5_LINE_LENGTH_IN                  0x00000418u
#define NVC6B5_LINE_COUNT                      0x0000041cu
#define NVC6B5_LAUNCH_DMA                      0x00000300u
#define NVC6B5_SET_SEMAPHORE_A                 0x00000240u
#define NVC6B5_SET_SEMAPHORE_B                 0x00000244u
#define NVC6B5_SET_SEMAPHORE_PAYLOAD           0x00000248u

#define NVC6B5_LAUNCH_DMA_FLUSH_ENABLE_TRUE           (1u << 2)
#define NVC6B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL            (0u << 12)
#define NVC6B5_LAUNCH_DMA_DST_TYPE_VIRTUAL            (0u << 13)
#define NVC6B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH     (1u << 7)
#define NVC6B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH     (1u << 8)
#define NVC6B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_NON_PIPELINED (2u << 0)
#define NVC6B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_ONE_WORD  (1u << 3)

/* Métodos de canal (class/clc56f.h). */
#define NVC56F_SET_OBJECT                    0x00000000u
#define NVC56F_GP_ENTRY__SIZE                8u
#define NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL 0u
#define NVC56F_GP_ENTRY1_SYNC_PROCEED        0u
#define NVC56F_GP_ENTRY1_LEVEL_MAIN          0u

#define NVC56F_DMA_INCR_OPCODE_VALUE         1u
#define NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD   4u

typedef char nv_memory_desc_size_check[sizeof(NV_MEMORY_DESC_PARAMS) == 24 ? 1 : -1];
typedef char nvc56f_control_size_check[sizeof(Nvc56fControl) == 512 ? 1 : -1];

/* ---- Compute Blackwell + QMD v05 (G4f) ------------------------------------
 * Referencias: `classes/compute/clcdc0.h`, `clcdc0qmd.h` (open-gpu-doc). */
#define BLACKWELL_COMPUTE_A       0x0000cdc0u
#define NVKM_RM_COMPUTE0          0xcdc00000u

#define GSP_QMD_VERSION_CURRENT   5u
#define GSP_QMD_INLINE_WORDS      96u   /* 384 B inline QMD (Blackwell v05) */

#define NVCDC0_SET_OBJECT                    0x00000000u
#define NVCDC0_SET_QMD_VERSION               0x00000288u
#define NVCDC0_SET_INLINE_QMD_ADDRESS_A      0x00000318u
#define NVCDC0_SET_INLINE_QMD_ADDRESS_B      0x0000031cu
#define NVCDC0_LOAD_INLINE_QMD_DATA(i)       (0x00000320u + (uint32_t)(i) * 4u)

#define NVCDC0_SET_INLINE_QMD_ADDRESS_A_INLINE_SIZE_INLINE_384  0x00000001u

#define NVCDC0_QMDV05_00_QMD_TYPE_GRID_CTA   0x00000002u

/* Campos del QMD v05, como pares (lo, hi) para `qmd_set_bits`. Transcritos de
 * `classes/compute/clcdc0qmd.h` (open-gpu-doc), donde vienen como MW(hi:lo).
 * OJO con los `_SHIFTED`: la dirección del programa va >>4, la del constant
 * bank >>6 (⇒ alineada a 64 B) y su tamaño >>4 (⇒ múltiplo de 16 B). */
#define QMDV05_QMD_TYPE                   151u, 153u
#define QMDV05_RELEASE_ENABLE0            288u, 288u
#define QMDV05_RELEASE_STRUCTURE_SIZE0    289u, 290u
#define QMDV05_RELEASE_MEMBAR_TYPE0       291u, 291u
#define QMDV05_RELEASE_SEM0_ADDR_LOWER    480u, 511u
#define QMDV05_RELEASE_SEM0_ADDR_UPPER    512u, 536u
#define QMDV05_RELEASE_SEM0_PAYLOAD_LOWER 544u, 575u
#define QMDV05_QMD_MAJOR_VERSION          468u, 471u
#define QMDV05_PROGRAM_ADDRESS_LOWER_S4   1024u, 1055u
#define QMDV05_PROGRAM_ADDRESS_UPPER_S4   1056u, 1076u
#define QMDV05_CTA_THREAD_DIMENSION0      1088u, 1103u
#define QMDV05_CTA_THREAD_DIMENSION1      1104u, 1119u
#define QMDV05_CTA_THREAD_DIMENSION2      1120u, 1127u
#define QMDV05_REGISTER_COUNT             1128u, 1136u
#define QMDV05_BARRIER_COUNT              1137u, 1141u
#define QMDV05_SHARED_MEMORY_SIZE_S7      1152u, 1162u
#define QMDV05_GRID_WIDTH                 1248u, 1279u
#define QMDV05_GRID_HEIGHT                1280u, 1295u
#define QMDV05_GRID_DEPTH                 1312u, 1327u
#define QMDV05_CBANK0_ADDR_LOWER_S6       1344u, 1375u
#define QMDV05_CBANK0_ADDR_UPPER_S6       1376u, 1394u
#define QMDV05_CBANK0_SIZE_S4             1395u, 1407u
#define QMDV05_CBANK0_VALID               1856u, 1856u
#define QMDV05_CBANK0_INVALIDATE          1859u, 1859u

#define NVCDC0_QMDV05_00_RELEASE_ENABLE_TRUE                        0x00000001u
#define NVCDC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD  0x00000001u
#define NVCDC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR           0x00000001u
#define NVCDC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE                 0x00000001u
#define NVCDC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE            0x00000001u
#define NVCDC0_QMDV05_00_QMD_MAJOR_VERSION_V05                      0x00000005u

typedef struct GspQmdV05 {
    NvU32 words[GSP_QMD_INLINE_WORDS];
} GspQmdV05;

typedef char gsp_qmd_v05_size_check[sizeof(GspQmdV05) == GSP_QMD_INLINE_WORDS * 4 ? 1 : -1];

#endif
