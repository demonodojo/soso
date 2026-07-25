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
 * **No existen en r570/nvrm/**: son compartidas con r535, sin divergencia de
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

/* Números de función de `rm/r570/nvrm/rpcfn.h`. */
#define NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL  76u
#define NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC   103u
#define NV_VGPU_MSG_FUNCTION_FREE            27u

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

/* Si un tamaño baila, RM lee los campos desplazados y responde cualquier cosa. */
typedef char rm_alloc_hdr_size_check[sizeof(rpc_gsp_rm_alloc) == 32 ? 1 : -1];
typedef char rm_ctrl_hdr_size_check[sizeof(rpc_gsp_rm_control) == 24 ? 1 : -1];
typedef char nv0000_size_check[sizeof(NV0000_ALLOC_PARAMETERS) == 120 ? 1 : -1];
typedef char nv0080_size_check[sizeof(NV0080_ALLOC_PARAMETERS) == 56 ? 1 : -1];
typedef char nv2080_size_check[sizeof(NV2080_ALLOC_PARAMETERS) == 4 ? 1 : -1];

#endif
