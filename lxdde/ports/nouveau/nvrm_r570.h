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

/* `rm/r570/nvrm/fifo.h` — evento RC_TRIGGERED (fn=0x1004). El journal va detrás
 * de `rcJournalBufferSize` palabras; aquí sólo decodificamos la cabecera fija. */
typedef struct rpc_rc_triggered_v17_02
{
    NvU32      nv2080EngineType;
    NvU32      chid;
    NvU32      gfid;
    NvU32      exceptLevel;
    NvU32      exceptType;
    NvU32      scope;
    NvU16      partitionAttributionId;
    NvU32      mmuFaultAddrLo;
    NvU32      mmuFaultAddrHi;
    NvU32      mmuFaultType;
    NvBool     bCallbackNeeded;
    NvU32      rcJournalBufferSize;
    NvU8       rcJournalBuffer[];
} rpc_rc_triggered_v17_02;

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

/* UPDATE_BAR_PDE (fn=70, `rpc_global_enums.h`). Es LA forma soportada de meter
 * una PDE en el vaspace de una apertura cuando manda GSP-RM: bajo lockdown el
 * host no puede escribir el registro que ata el bloque de instancia (se escribe
 * y se relee cero, medido en la GB205 el 2026-08-02), así que se le pide a RM.
 *
 * `r535_bar_bar2_update_pde` (`nvkm/subdev/bar/r535.c:46-60`) lo usa para BAR2
 * con `entryValue = (addr >> 4) | 2` —el comentario de upstream dice literalmente
 * «PD3 entry format!»— y `entryLevelShift = 47`, o sea que la entrada que RM
 * escribe es del nivel cuyas entradas cubren 2^47 y apunta a la tabla PD2 del
 * driver. Para BAR1 es el mismo mensaje con otro `barType`. */
#define NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE 70u

/* ---- Journal de RC (r570: `src/common/sdk/nvidia/inc/rmcd.h`) --------------
 *
 * Lo que RM adjunta al RC_TRIGGERED no es opaco: empieza por una cabecera
 * `NVCD_RECORD` con el TIPO de registro, y para el tipo 147 (`RmRcDiagReport`)
 * el cuerpo son entradas `{offset, tag, value, attribute}` — o sea **el volcado
 * de registros que RM leyó al saltar la excepción**, con dirección y valor.
 *
 * Estaba llegando desde el primer ciclo y lo imprimíamos como hexadecimal a
 * pelo. Con la cabeza que capturó el silicio el 2026-08-01 (`0cc89301`) sale
 * grupo=1 tipo=0x93=147 tamaño=0x0cc8: encaja exactamente. */
typedef struct NVCD_RECORD_hdr
{
    NvU8                    cRecordGroup;
    NvU8                    cRecordType;
    NvU16                   wRecordSize;
} NVCD_RECORD_hdr;

/* Cabecera común que RM pide poner al principio de todo registro del journal. */
typedef struct RmRCCommonJournal_RECORD_hdr
{
    NVCD_RECORD_hdr         Header;
    NvU32                   GPUTag;
    NvU64                   CPUTag;
    NvU64                   timeStamp;
    NvU64                   stateMask;
    NvU64                   pNext;      /* puntero del host: aquí sólo se salta */
} RmRCCommonJournal_RECORD_hdr;

typedef struct RmRcDiagRecordEntry
{
    NvU32                   offset;     /* registro leído */
    NvU32                   tag;
    NvU32                   value;      /* lo que valía */
    NvU32                   attribute;
} RmRcDiagRecordEntry;

/* Cuerpo del tipo 147, sin el array (se recorre a mano sobre el búfer). */
typedef struct RmRcDiag_RECORD_hdr
{
    NvU16                   idx;
    NvU32                   timeStamp;
    NvU16                   type;
    NvU32                   flags;
    NvU16                   count;
    NvU32                   owner;
    NvU32                   processId;
} RmRcDiag_RECORD_hdr;

#define RMCD_RECORD_RCDIAGREPORT 147u
#define RMCD_RECORD_NOCATREPORT  149u

typedef char rcdiag_entry_size_check[sizeof(RmRcDiagRecordEntry) == 16 ? 1 : -1];

#define NV_RPC_UPDATE_PDE_BAR_1 0u
#define NV_RPC_UPDATE_PDE_BAR_2 1u

typedef struct UpdateBarPde_v15_00
{
    NvU32                   barType;
    NvU64                   entryValue;
    NvU64                   entryLevelShift;
} UpdateBarPde_v15_00;

typedef struct rpc_update_bar_pde_v15_00
{
    UpdateBarPde_v15_00     info;
} rpc_update_bar_pde_v15_00;

/* 24 B con alineación natural: barType (4) + hueco (4) + dos NvU64. Si esto
 * baila, RM lee el barType donde no es y actualizaría la apertura equivocada. */
typedef char update_bar_pde_size_check[
    sizeof(rpc_update_bar_pde_v15_00) == 24 ? 1 : -1];

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

/* Catálogo de clases del chip. **V2 y no la de siempre**: la primera
 * (`GET_CLASSLIST`, 0x800201) devuelve la lista por un `NvP64 classList` que
 * apunta a memoria del llamante — un puntero que al otro lado del RPC no
 * significa nada. La V2 lleva el array dentro de los params, que es lo que
 * cabe en un mensaje. 404 B de ida, dentro de RM_PARAMS_MAX.
 *
 * **100 y no 200.** El 200 es el valor de la rama `main` de
 * open-gpu-kernel-modules; en el tag **570.144**, que es el firmware de esta
 * tarjeta, son 100. Con 200 los params medían 804 B y RM contestaba
 * INVALID_ARGUMENT (0x1f) — dos ciclos de hardware con el catálogo caído y las
 * tres clases eligiéndose sin él (2026-07-27 y 28). Ver la regla del bloque de
 * NVA06F más abajo: el tag, no main. */
#define NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2     0x800292u
#define NV0080_CTRL_GPU_CLASSLIST_MAX_SIZE       100u

typedef struct NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS
{
    NvU32                   numClasses;
    NvU32                   classList[NV0080_CTRL_GPU_CLASSLIST_MAX_SIZE];
} NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS;

typedef char nv0080_classlist_size_check[
    sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS) == 404 ? 1 : -1];

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
 * (open-gpu-kernel-modules), `include/nvif/class.h` de nouveau.
 *
 * **GB20x NO comparte la ruta Ampere** — eso era una suposición y estaba mal.
 * `nvkm/subdev/gsp/rm/gb20x.c` de nouveau dice para este chip:
 * canal = `BLACKWELL_CHANNEL_GPFIFO_B`, CE = `BLACKWELL_DMA_COPY_B`,
 * compute = `BLACKWELL_COMPUTE_B`. Ninguna de las tres es la que pedíamos.
 *
 * Aun así aquí no se elige por familia: se pregunta. `GET_CLASSLIST_V2` da la
 * lista exacta de clases del chip y `gsp_rm_class_pick` coge la primera de la
 * lista de candidatas que RM reconozca. Una tabla por familia es otra tabla que
 * se queda vieja con el siguiente chip; el catálogo lo dice la tarjeta. */

/* **8, no 32.** `nvlimits.h` de open-gpu-kernel-modules define los dos macros
 * pegados uno al otro:
 *
 *     #define NV_MAX_DEVICES                32
 *     #define NV_MAX_SUBDEVICES             8
 *
 * y aquí se había copiado el de arriba. `r570/nvrm/fifo.h` de nouveau —el
 * fichero que esta sección espeja, con su excerpt de 570.144— dice 8 para esta
 * struct exacta.
 *
 * No es un macro decorativo: aparece dos veces en NV_CHANNEL_ALLOC_PARAMS
 * (`hUserdMemory[]` y `userdOffset[]`), así que el 32 metía 288 B de más y
 * corría TODO lo que viene detrás —los cuatro descriptores de memoria,
 * `internalFlags`, `hPhysChannelGroup`— esos mismos 288 B. RM leía instanceMem
 * donde nosotros ya no escribíamos nada y contestaba INVALID_PARAMETER (0x3b) a
 * las once variantes de la sonda por igual, que es justo la firma de un layout
 * mal: si fallan todas las hipótesis de campo, lo que está mal no es un campo.
 * (2026-07-28, mismo mordisco que el `hHandleVASpace` del día anterior.) */
#define NV_MAX_SUBDEVICES 8u

#define AMPERE_CHANNEL_GPFIFO_A    0x0000c56fu
#define AMPERE_CHANNEL_GPFIFO_B    0x0000c76fu
#define HOPPER_CHANNEL_GPFIFO_A    0x0000c86fu
#define BLACKWELL_CHANNEL_GPFIFO_A 0x0000c96fu
#define BLACKWELL_CHANNEL_GPFIFO_B 0x0000ca6fu

#define AMPERE_DMA_COPY_A          0x0000c6b5u
#define AMPERE_DMA_COPY_B          0x0000c7b5u
#define HOPPER_DMA_COPY_A          0x0000c8b5u
#define BLACKWELL_DMA_COPY_A       0x0000c9b5u
#define BLACKWELL_DMA_COPY_B       0x0000cab5u

/* Handle, no clase: sale de un espacio propio y da igual qué DMA_COPY sea. */
#define NVKM_RM_CE0                0xc6b50000u

/* `NV2080_ENGINE_TYPE_COPY0` de `rm/r535/nvrm/engine.h`. **9, no 6**: la tabla
 * empieza en GR0=1 y reserva OCHO huecos para los GR (GR0..GR7 = 1..8), así que
 * el primer CE cae en el 9. El 6 que había aquí es `NV2080_ENGINE_TYPE_GR5` —un
 * motor gráfico que esta tarjeta no tiene—, y pedir un canal sobre él es
 * exactamente lo que RM contesta con NV_ERR_INVALID_PARAMETER (0x3b,
 * comprobado en HW el 2026-07-28 con el resto de los campos ya correctos).
 * `RM_ENGINE_TYPE_COPY0` del enum interno vale 9 también, así que aquí las dos
 * numeraciones coinciden y no hay trampa que valga. */
#define NV2080_ENGINE_TYPE_COPY0   9u

/* Y GR0 es el 1, la primera entrada útil de esa misma tabla (el 0 es NULL). Hace
 * falta porque un objeto de compute NO se puede colgar de un canal de copia: RM
 * contesta INVALID_CLASS (0x22) a `BLACKWELL_COMPUTE_B` sobre el canal de COPY0
 * aunque la clase sea la correcta y el mismo canal acepte `BLACKWELL_DMA_COPY_B`
 * (HW, 2026-07-28). El compute necesita su propio canal atado a GR0. */
#define NV2080_ENGINE_TYPE_GR0     1u

/* `addressSpace` de NV_MEMORY_DESC_PARAMS es el enum `NV_ADDRESS_SPACE` de RM,
 * y NO el `FLAGS_APERTURE` de SET_PAGE_DIRECTORY que hay 50 líneas más arriba
 * (donde SYSMEM_COH sí es 1 y VIDMEM 0). Confundirlos costó el `4` que había
 * aquí como "SYSMEM_COHERENT": en `r535_chan_alloc` upstream pone 2 en los tres
 * descriptores que viven en VRAM (instance, USERD, ramfc) y 1 en el único que
 * vive en sysmem (mthdbuf), así que 1=sysmem y 2=VRAM. El 4 no es sysmem. */
#define NV_ADDRESS_SPACE_SYSMEM           1u
#define NV_ADDRESS_SPACE_FBMEM            2u
/* Valores de `nv_memory_type.h` (OGKM). Los nombres viejos estaban invertidos
 * respecto a upstream: r535 pone cacheAttrib=1 en VRAM (= UNCACHED aquí). */
#define NV_CACHE_ATTR_CACHED              0u
#define NV_CACHE_ATTR_UNCACHED            1u
#define NV_CACHE_ATTR_DEFAULT             6u

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

/* Estos asserts decían "ancla contra upstream, no contra nosotros mismos" y era
 * MENTIRA: el 656 y el 432 se habían calculado con nuestro `NV_MAX_SUBDEVICES`,
 * o sea que cuadraban con el error y lo bendecían. Un assert derivado de la
 * misma constante que quieres comprobar no comprueba nada.
 *
 * Los de ahora salen de sumar a mano los campos con NV_MAX_SUBDEVICES=8 de
 * `nvlimits.h`, y el tercero es el que faltaba: `hUserdMemory` está ANTES de los
 * dos arrays, así que su offset 32 no se movió ni con el 32 ni con el 8 — el
 * único ancla que había del lado malo era ciego a propósito. `instanceMem` está
 * después: ahí es donde se ve.
 *
 *   hObjectError 0, hObjectBuffer 4, gpFifoOffset 8, gpFifoEntries 16,
 *   flags 20, hContextShare 24, hVASpace 28, hUserdMemory[8] 32..64,
 *   userdOffset[8] 64..128, engineType 128, cid 132, subDeviceId 136,
 *   hObjectEccError 140, instanceMem 144, userdMem 168, ramfcMem 192,
 *   mthdbufMem 216, hPhysChannelGroup 240, internalFlags 244,
 *   errorNotifierMem 248, eccErrorNotifierMem 272, ProcessID 296,
 *   SubProcessID 300, encryptIv 304, decryptIv 316, hmacNonce 328,
 *   tpcConfigID 360, y el relleno a múltiplo de 8 → 368.
 */
typedef char nv_channel_alloc_size_check[
    sizeof(NV_CHANNEL_ALLOC_PARAMS) == 368 ? 1 : -1];
typedef char nv_channel_alloc_userd_off_check[
    offsetof(NV_CHANNEL_ALLOC_PARAMS, hUserdMemory) == 32 ? 1 : -1];
typedef char nv_channel_alloc_instmem_off_check[
    offsetof(NV_CHANNEL_ALLOC_PARAMS, instanceMem) == 144 ? 1 : -1];
/* El otro multiplicador de la cuenta de arriba: cuatro descriptores entre
 * `instanceMem` y `hPhysChannelGroup`. Si esto no son 24, el 368 tampoco vale. */
typedef char nv_memory_desc_size_check[
    sizeof(NV_MEMORY_DESC_PARAMS) == 24 ? 1 : -1];

#define NVOS04_FLAGS_CHANNEL_TYPE_PHYSICAL  0x00000000u
#define NVOS04_FLAGS_CHANNEL_CLIENT_MAP_FIFO  (1u << 24)

/* `flags` de NVOS04 es un campo de bits, y casi todos los subcampos que pone
 * `r535_chan_alloc` valen FALSE=0 — o sea que un `flags` a cero es *casi*
 * correcto. Los dos que NO son cero:
 *
 * - PRIVILEGED_CHANNEL (5:5), que va emparejado con el PRIVILEGE de
 *   `internalFlags`: upstream mueve los dos según su `priv`, y pedir ADMIN en
 *   uno mientras el otro dice FALSE es una combinación que no manda nunca.
 * - CHANNEL_USERD_INDEX_PAGE_FIXED (21:21), que upstream pone a TRUE siempre,
 *   con PAGE_VALUE = chid / CHID_PER_USERD e INDEX_VALUE = chid %
 *   CHID_PER_USERD (CHID_PER_USERD son 8: ocho USERD de 0x200 B por página).
 *   Con chid=0 los dos valores son 0 y el único bit que queda es este.
 *
 * **Y ahí estaba el fallo del segundo canal** (2026-07-28): esta regla estaba
 * escrita aquí y el código la ignoraba, poniendo INDEX_VALUE=0 y PAGE_VALUE=0
 * **fijos**. Con un solo canal da igual —chid 0 son ceros— pero el segundo pedía
 * el MISMO slot de USERD que el primero teniéndolo declarado como fijo, y su
 * `GSP_RM_ALLOC` volvió con `NO_MEMORY`. Estos dos campos son la vía por la que
 * el llamante le dice a RM qué chid quiere: en `r535_chan_alloc` el chid lo
 * elige nouveau con su propio asignador y sólo viaja hasta RM dentro de estos
 * bits. Verbatim de upstream (master, `rm/r535/fifo.c`):
 *
 *     const int userd_p = chid / CHID_PER_USERD;
 *     const int userd_i = chid % CHID_PER_USERD;
 *     args->flags |= NVVAL(NVOS04, FLAGS, CHANNEL_USERD_INDEX_VALUE, userd_i);
 *     args->flags |= NVDEF(NVOS04, FLAGS, CHANNEL_USERD_INDEX_FIXED, FALSE);
 *     args->flags |= NVVAL(NVOS04, FLAGS, CHANNEL_USERD_INDEX_PAGE_VALUE, userd_p);
 *     args->flags |= NVDEF(NVOS04, FLAGS, CHANNEL_USERD_INDEX_PAGE_FIXED, TRUE);
 */
#define NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE            (1u << 5)
#define NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(i)       (((i) & 0x7u) << 8)
/* INDEX_FIXED (11:11) va a FALSE, que es 0. Existe como define para que se vea que
 * es una decisión de upstream y no un campo que se nos olvidó: el que va fijo es
 * la PÁGINA, no el índice dentro de ella. */
#define NVOS04_FLAGS_CHANNEL_USERD_INDEX_FIXED_FALSE    0u
#define NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_VALUE(p)  (((p) & 0x1ffu) << 12)
#define NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED_TRUE (1u << 21)
#define NV_CHID_PER_USERD                               8u

/* El tamaño del method buffer no se inventa: se le pregunta a RM. Upstream lo
 * hace en `r535_fifo_ctor` con este control sobre el SUBDEVICE, y guarda el
 * resultado en `fifo->rm.mthdbuf_size` para dárselo a cada canal. */
#define NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE  0x20802a08u

typedef struct NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS {
    NvU32 size;
} NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS;

/* ---- Qué motores tiene el chip, y en qué runlist ----------------------------
 *
 * `engineType` del canal sale de aritmética sobre la tabla de `engine.h`
 * (GR0..GR7 = 1..8, luego COPY0 = 9). La aritmética está bien, pero no dice si
 * ESTE chip tiene ese motor, ni si está en una runlist a la que un canal físico
 * pueda engancharse. Eso se pregunta, igual que las clases: `r535_fifo_ctor` de
 * nouveau usa estos dos controles sobre el subdevice antes de tocar un canal.
 *
 * `GET_ENGINES_V2` da la lista de `NV2080_ENGINE_TYPE_*` que RM expone.
 * `GET_DEVICE_INFO_TABLE` da la topología: por motor, sus PBDMA y —lo que aquí
 * importa— un `engineName` en texto. Ese nombre es además el autochequeo del
 * layout: si sale legible ("COPY0", "GR0"), la transcripción de la struct es
 * buena; si sale basura, está desplazada y no hay que creerse el resto.
 *
 * Referencias: `ctrl2080gpu.h` y `ctrl2080fifo.h` de open-gpu-kernel-modules
 * (leídos el 2026-07-28; el 0x54 de abajo es suyo, NO 0x34). */
#define NV2080_CTRL_CMD_GPU_GET_ENGINES_V2   0x20800170u
#define NV2080_GPU_MAX_ENGINES_LIST_SIZE     0x54u    /* 84 */

typedef struct NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS {
    NvU32 engineCount;
    NvU32 engineList[NV2080_GPU_MAX_ENGINES_LIST_SIZE];
} NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS;

typedef char nv2080_engines_v2_size_check[
    sizeof(NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS) == 340 ? 1 : -1];

#define NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE  0x20801112u
#define NV2080_CTRL_FIFO_DEVICE_INFO_MAX_ENTRIES    32u
#define NV2080_CTRL_FIFO_DEVICE_INFO_DATA_TYPES     16u
#define NV2080_CTRL_FIFO_DEVICE_INFO_MAX_PBDMA      2u
#define NV2080_CTRL_FIFO_DEVICE_INFO_MAX_NAME_LEN   16u

typedef struct NV2080_CTRL_FIFO_DEVICE_ENTRY {
    NvU32 engineData[NV2080_CTRL_FIFO_DEVICE_INFO_DATA_TYPES];
    NvU32 pbdmaIds[NV2080_CTRL_FIFO_DEVICE_INFO_MAX_PBDMA];
    NvU32 pbdmaFaultIds[NV2080_CTRL_FIFO_DEVICE_INFO_MAX_PBDMA];
    NvU32 numPbdmas;
    char  engineName[NV2080_CTRL_FIFO_DEVICE_INFO_MAX_NAME_LEN];
} NV2080_CTRL_FIFO_DEVICE_ENTRY;

/* La tabla viene paginada: 32 entradas por llamada, `baseIndex` dice por dónde
 * seguir y `bMore` si queda más. Los índices de `engineData` viven en un enum
 * aparte que ctrl2080fifo.h no trae, así que aquí NO se nombran: se vuelcan los
 * 16 words crudos y ya se decodificarán cuando haga falta. Inventar nombres para
 * los índices es cómo se llega a un GR5 disfrazado de COPY0. */
typedef struct NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS {
    NvU32                         baseIndex;
    NvU32                         numEntries;
    NvBool                        bMore;
    NV2080_CTRL_FIFO_DEVICE_ENTRY entries[NV2080_CTRL_FIFO_DEVICE_INFO_MAX_ENTRIES];
} NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS;

typedef char nv2080_fifo_devinfo_entry_size_check[
    sizeof(NV2080_CTRL_FIFO_DEVICE_ENTRY) == 100 ? 1 : -1];
/* `bMore` es NvU8 en el offset 8; `entries` se alinea a 4 y cae en el 12, no en
 * el 16. Ese hueco de 3 B es el sitio exacto donde un NvBool tomado por NvU32
 * desplazaría la tabla entera. */
typedef char nv2080_fifo_devinfo_entries_off_check[
    offsetof(NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS, entries) == 12 ? 1 : -1];
typedef char nv2080_fifo_devinfo_size_check[
    sizeof(NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS) == 3212 ? 1 : -1];

/* ---- Arrancar el canal: BIND + SCHEDULE + doorbell -------------------------
 *
 * Reservar el canal NO lo pone a correr, y esto faltaba entero: el RM_ALLOC
 * pasaba, el CE se colgaba del canal, y la copia sysmem → VRAM no señalizaba
 * nunca porque nadie había metido el canal en la runlist ni había pateado el
 * host (2026-07-28). Upstream lo hace en tres pasos, todos sobre el objeto del
 * CANAL (no sobre el subdevice):
 *
 *   1. `NVA06F_CTRL_CMD_BIND` con el engineType — ata el canal a su motor.
 *   2. `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` con bEnable=1 (`r535_chan_start`).
 *   3. El doorbell, que NO es un RPC sino una escritura de registro.
 *
 * Referencias: `rm/r535/fifo.c` de nouveau para los dos controles,
 * `ctrl/ctrla06f/ctrla06fgpfifo.h` y `ctrl/ctrlc36f.h` de
 * open-gpu-kernel-modules para los valores (leídos el 2026-07-28).
 *
 * REGLA, pagada con un ciclo de hardware: las structs se leen del **tag
 * 570.144** de open-gpu-kernel-modules, no de `main`. El firmware de esta
 * tarjeta es 570.144 y RM compara el `paramsSize` contra la struct con la que se
 * compiló ÉL; un campo añadido en una versión posterior no es un detalle
 * cosmético, es un INVALID_ARGUMENT. Ya cayeron dos así el 2026-07-28: el
 * `bSkipEnable` de aquí abajo y el CLASSLIST_MAX_SIZE del catálogo (100 en
 * 570.144, 200 en main). Es la misma razón por la que los headers de nouveau
 * dicen "Excerpt of RM headers from .../tree/570.144" y no "from main". */
#define NVA06F_CTRL_CMD_GPFIFO_SCHEDULE  0xa06f0103u
#define NVA06F_CTRL_CMD_BIND             0xa06f0104u

typedef struct NVA06F_CTRL_BIND_PARAMS {
    NvU32 engineType;
} NVA06F_CTRL_BIND_PARAMS;

/* DOS NvBool, o sea dos BYTES. Aquí hubo tres —copiados de la rama `main` de
 * open-gpu-kernel-modules— y RM contestó INVALID_ARGUMENT (0x1f) al SCHEDULE en
 * hardware: `bSkipEnable` es un campo POSTERIOR a 570.144, y el GSP de esta
 * tarjeta valida el paramsSize contra la struct con la que se compiló él.
 *
 * De ahí la regla del bloque de arriba: el tag, no `main`. */
typedef struct NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS {
    NvBool bEnable;
    NvBool bSkipSubmit;
} NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS;

typedef char nva06f_schedule_size_check[
    sizeof(NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS) == 2 ? 1 : -1];
typedef char nva06f_bind_size_check[
    sizeof(NVA06F_CTRL_BIND_PARAMS) == 4 ? 1 : -1];

/* GET_WORK_SUBMIT_TOKEN da runlist+chid para la vía interna de GSP-RM. El valor
 * que hay que escribir en NV_VFN_DOORBELL lo construye el driver: en gb20x lleva
 * además RUNLIST_DOORBELL_ENABLE (bit 30) — ver gsp_chan_doorbell_kick() y
 * gb202/dev_vm.h. Referencia: gb202_chan_doorbell_handle, tu102_chan_doorbell_handle. */
#define NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN  0xc36f0108u

typedef struct NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS {
    NvU32 workSubmitToken;
} NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS;

/* Registro del doorbell, en BAR0. `nvkm_vfn_new_` monta
 * `addr.user = addr.priv + func->user.addr`, y el kick de `tu102_chan_start` es
 * `nvkm_wr32(device, device->vfn->addr.user + 0x0090, token)`.
 *
 * Con `ga100_vfn`: priv = 0xb80000, user = +0x030000 → 0xbb0000, doorbell en
 * 0xbb0090. Que esto valga para GB20x no es una suposición de familia: en el
 * `nvkm/subdev/vfn/Kbuild` de nouveau **no hay ningún vfn más nuevo que
 * ga100.c** (base, uvfn, gv100, tu102, ga100, r535), así que Ampere→Blackwell
 * comparten el mismo. Y el `r535_vfn` del camino GSP, que es el que aplica aquí,
 * mantiene `user = { 0x030000, 0x010000 }`. */
#define NV_VFN_USERMODE_BASE     0xbb0000u
#define NV_VFN_DOORBELL          (NV_VFN_USERMODE_BASE + 0x0090u)

/* Campos de NV_VIRTUAL_FUNCTION_DOORBELL (gb202/dev_vm.h, tag 570.144). Turing
 * solo tiene VECTOR+RUNLIST_ID; Blackwell añade RUNLIST_DOORBELL en el bit 30. */
#define NV_VF_DOORBELL_VECTOR_MASK           0x00000fffu
#define NV_VF_DOORBELL_RUNLIST_ID_MASK       0x007f0000u
#define NV_VF_DOORBELL_RUNLIST_DOORBELL_ENABLE  0x40000000u

/* Bloque de instancia del canal. Upstream (`r535_chan_alloc`) apunta
 * `instanceMem` al bloque entero y `ramfcMem` a sus primeros 0x200 B, los dos
 * en VRAM. */
#define GSP_CHAN_INST_SIZE   0x1000u
#define GSP_CHAN_RAMFC_SIZE  0x200u

/* Tamaño del USERD **del chip**, no de la página que lo aloja: `gv100_chan_userd`
 * (que usan tu102, ga100 y por herencia Blackwell) declara `.size = 0x200`, y
 * cuadra con CHID_PER_USERD=8. Declararle a RM los 4096 de la página entera era
 * pedirle un USERD ocho veces el que tiene el chip. */
#define GSP_CHAN_USERD_HW_SIZE  0x200u

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
/* 9:9 en `cla0b5.h`. Con él, la copia son `LINE_COUNT` líneas de `LINE_LENGTH_IN`
 * separadas por `PITCH`: con pitch = longitud de línea salen contiguas, que es
 * como upstream mueve un buffer entero en un solo LAUNCH_DMA. */
#define NVC6B5_LAUNCH_DMA_MULTI_LINE_ENABLE_TRUE           (1u << 9)

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

/* ---- Compute Blackwell + QMD v05 (G4f/G4h) --------------------------------
 * Referencias: `classes/compute/clcec0.h`, `clcec0qmd.h` (open-gpu-doc, gb20x
 * clase B). Los campos del QMD coinciden con `clcdc0qmd.h` (clase A); cambia la
 * clase, no el encoding del descriptor.
 *
 * GB20x usa la **B** (`BLACKWELL_COMPUTE_B`, `rm/gb20x.c`); la A es de GB100.
 * El lanzamiento sigue a Mesa/nvk Blackwell: QMD en memoria + WFI del canal +
 * SEND_PCAS_A + SEND_SIGNALING_PCAS2_B — no el camino inline (mal codificado y
 * sin paridad en ningún driver real, ciclo 2026-07-29). */
#define AMPERE_COMPUTE_A          0x0000c6c0u
#define AMPERE_COMPUTE_B          0x0000c7c0u
#define ADA_COMPUTE_A             0x0000c9c0u
#define HOPPER_COMPUTE_A          0x0000cbc0u
#define BLACKWELL_COMPUTE_A       0x0000cdc0u
#define BLACKWELL_COMPUTE_B       0x0000cec0u
#define NVKM_RM_COMPUTE0          0xcdc00000u

#define GSP_QMD_VERSION_CURRENT   5u
#define GSP_QMD_INLINE_WORDS      96u   /* 384 B QMD v05 (Blackwell) */

#define NVCEC0_SET_OBJECT                    0x00000000u
#define NVCEC0_SEND_PCAS_A                   0x000002b4u
#define NVCEC0_SEND_SIGNALING_PCAS2_B        0x000002c0u
#define NVCEC0_SEND_SIGNALING_PCAS2_B_PCAS_ACTION_INVALIDATE_COPY_SCHEDULE 0x3u

/* Stall del command streamer antes del dispatch en Blackwell (Mesa/nvk). */
#define NVC86F_WFI                           0x00000078u

#define NVCEC0_QMDV05_00_QMD_TYPE_GRID_CTA   0x00000002u

/* Campos del QMD v05, como pares (lo, hi) para `qmd_set_bits`. Transcritos de
 * `classes/compute/clcec0qmd.h` (open-gpu-doc), donde vienen como MW(hi:lo).
 * OJO con los `_SHIFTED`: la dirección del programa va >>4, la del constant
 * bank >>6 (⇒ alineada a 64 B) y su tamaño >>4 (⇒ múltiplo de 16 B). */
#define QMDV05_QMD_TYPE                   151u, 153u
#define QMDV05_QMD_GROUP_ID               144u, 149u
#define QMDV05_API_VISIBLE_CALL_LIMIT     456u, 456u
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

#define NVCEC0_QMDV05_00_RELEASE_ENABLE_TRUE                        0x00000001u
#define NVCEC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD  0x00000001u
#define NVCEC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR           0x00000001u
#define NVCEC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE                 0x00000001u
#define NVCEC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE            0x00000001u
#define NVCEC0_QMDV05_00_QMD_MAJOR_VERSION_V05                      0x00000005u
#define NVCEC0_QMDV05_00_API_VISIBLE_CALL_LIMIT_NO_CHECK            0x00000001u

/* Alias NVCDC0_* → NVCEC0_* (mismo encoding; el código previo usaba CDC0). */
#define NVCDC0_SET_OBJECT                    NVCEC0_SET_OBJECT
#define NVCDC0_QMDV05_00_QMD_TYPE_GRID_CTA   NVCEC0_QMDV05_00_QMD_TYPE_GRID_CTA
#define NVCDC0_QMDV05_00_RELEASE_ENABLE_TRUE                        NVCEC0_QMDV05_00_RELEASE_ENABLE_TRUE
#define NVCDC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD  NVCEC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD
#define NVCDC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR           NVCEC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR
#define NVCDC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE                 NVCEC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE
#define NVCDC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE            NVCEC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE
#define NVCDC0_QMDV05_00_QMD_MAJOR_VERSION_V05                      NVCEC0_QMDV05_00_QMD_MAJOR_VERSION_V05

typedef struct GspQmdV05 {
    NvU32 words[GSP_QMD_INLINE_WORDS];
} GspQmdV05;

typedef char gsp_qmd_v05_size_check[sizeof(GspQmdV05) == GSP_QMD_INLINE_WORDS * 4 ? 1 : -1];

/* ---- Contexto de GR: consulta y promoción (G4f) ---------------------------
 *
 * Un canal de GR no ejecuta nada hasta que su contexto está **promocionado**: RM
 * necesita saber dónde viven los búferes de contexto del gráfico (el principal, el
 * de parches, los constant buffers globales, el mapa de acceso privilegiado…). El
 * driver los reserva y los mapea, y se los entrega con un solo control.
 *
 * Transcrito el 2026-07-28 de dos sitios, no de memoria: la consulta y sus params
 * de `rm/r570/nvrm/gr.h` de nouveau (que es el juego de 570.144, el de estos
 * blobs), y `NV2080_CTRL_GPU_PROMOTE_CTX_*` del header de la SDK en
 * open-gpu-kernel-modules 570.144 (`ctrl2080gpu.h`). Los asserts de tamaño y
 * offset de abajo son la defensa: el `hHandleVASpace` inventado del canal
 * desplazaba TODO lo que venía detrás y RM contestaba con un error que hablaba de
 * otra cosa. Un layout mal pero coherente consigo mismo es invisible desde dentro.
 */
#define NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO 0x20800a32u
#define NV2080_CTRL_INTERNAL_GR_MAX_ENGINES                          8u
#define NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_COUNT 0x1au

typedef struct NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_BUFFER_INFO {
    NvU32 size;
    NvU32 alignment;
} NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_BUFFER_INFO;

typedef struct NV2080_CTRL_INTERNAL_STATIC_GR_CONTEXT_BUFFERS_INFO {
    NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_BUFFER_INFO
        engine[NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_COUNT];
} NV2080_CTRL_INTERNAL_STATIC_GR_CONTEXT_BUFFERS_INFO;

typedef struct NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS {
    NV2080_CTRL_INTERNAL_STATIC_GR_CONTEXT_BUFFERS_INFO
        engineContextBuffersInfo[NV2080_CTRL_INTERNAL_GR_MAX_ENGINES];
} NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS;

/* Índices de `engine[]`: son ids de propiedad de contexto
 * (`NV0080_CTRL_FIFO_GET_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_*`), NO los bufferId
 * de la promoción. Los dos juegos de números existen y no coinciden: el mapa de
 * `gsp_grctx.c` traduce de unos a otros. */
#define NV0080_CTX_PROP_GRAPHICS                 0x00u
#define NV0080_CTX_PROP_GRAPHICS_PAGEPOOL        0x0du
#define NV0080_CTX_PROP_GRAPHICS_PATCH           0x10u
#define NV0080_CTX_PROP_GRAPHICS_BUNDLE_CB       0x11u
#define NV0080_CTX_PROP_GRAPHICS_ATTRIBUTE_CB    0x13u
#define NV0080_CTX_PROP_GRAPHICS_RTV_CB_GLOBAL   0x14u
#define NV0080_CTX_PROP_GRAPHICS_FECS_EVENT      0x17u
#define NV0080_CTX_PROP_GRAPHICS_PRIV_ACCESS_MAP 0x18u

#define NV2080_CTRL_CMD_GPU_PROMOTE_CTX          0x2080012bu
#define NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES 16u

#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN                         0u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PM                           1u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PATCH                        2u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_BUFFER_BUNDLE_CB             3u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PAGEPOOL                     4u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB                 5u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_RTV_CB_GLOBAL                6u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_GFXP_POOL                    7u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_GFXP_CTRL_BLK                8u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_FECS_EVENT                   9u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP              10u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP 11u
#define NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_GLOBAL_PRIV_ACCESS_MAP       12u

/* `physAttr` vale 4 en la promoción de upstream y sólo se rellena cuando el búfer
 * se inicializa; el resto de los campos de la entrada se quedan a cero. */
#define NV2080_CTRL_GPU_PROMOTE_CTX_PHYS_ATTR_DEFAULT 4u

typedef struct NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY {
    NvU64 gpuPhysAddr;
    NvU64 gpuVirtAddr;
    NvU64 size;
    NvU32 physAttr;
    NvU16 bufferId;
    NvU8  bInitialize;
    NvU8  bNonmapped;
} NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY;

typedef struct NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS {
    NvU32    engineType;
    NvHandle hClient;
    NvU32    ChID;
    NvHandle hChanClient;
    NvHandle hObject;
    NvHandle hVirtMemory;
    NvU64    virtAddress;
    NvU64    size;
    NvU32    entryCount;
    /* El array va alineado a 8 en el header de la SDK (`NV_DECLARE_ALIGNED`), y con
     * `entryCount` en el offset 40 eso mete **4 bytes de relleno** antes: empieza
     * en el 48, no en el 44. Ese hueco es exactamente la clase de detalle que
     * desplaza medio struct sin que nada se queje, y por eso está en un assert. */
    NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY
        promoteEntry[NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES];
} NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS;

typedef char nv2080_grctx_info_size_check[
    sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS) == 1664 ? 1 : -1];
typedef char nv2080_promote_entry_size_check[
    sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY) == 32 ? 1 : -1];
typedef char nv2080_promote_params_size_check[
    sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS) == 560 ? 1 : -1];
typedef char nv2080_promote_entry_off_check[
    offsetof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS, promoteEntry) == 48 ? 1 : -1];
typedef char nv2080_promote_virtaddr_off_check[
    offsetof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS, virtAddress) == 24 ? 1 : -1];

#endif
