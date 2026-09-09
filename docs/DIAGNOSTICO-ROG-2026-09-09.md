# ROG: GPU GA107 y WiFi AX200 — diagnóstico (9 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-09-run{3,4}/`. ESP desmontada al terminar.

| Campo | run3 | run4 (último USB) |
|---|---|---|
| Kernel USB | 0.2.2 (669afd2c1-dirty) | **0.2.2 (4a40cbca7)** |
| Flush / uptime | #10, 364 s | **#8, 280 s** |
| Hardware | 10de:249c + 8086:2723 + 10ec:8168 | igual |
| sosh | OK | OK |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | 6.6.32 — `mvm/fw.c`, `fw/api/nvm-reg.h`, `fw/api/commands.h` |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf` — `tu102.c`, `rm/r570/rm.c`, `rm/r535/gsp.c` |

Hostchecks tras corrección: `l6-iwl-fw-hostcheck.sh` OK,
`l6-g3-gsp-hostcheck.sh` OK (heap Ampere **128 MiB**, pmu=0).

## Tabla de etapas (run4)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh — escribe 'help'` | OK |
| GPU: FWSEC-FRTS + booter | `booter_load boot ok`, WPR2 OK | OK (Falcon) |
| GPU: GSP-RM / RPC | `RISC-V inactivo cpuctl=0x10`, `GSP=fallo` | **FAIL** |
| GPU: pool VRAM | `pool VRAM=no` | **FAIL** |
| WiFi: ALIVE | `UCODE_ALIVE_NTFY` | OK |
| WiFi: INIT_EXTENDED_CFG | rsp grp=2 id=0x03 | OK |
| WiFi: NVM | timeout grp=12 id=0x88 / 0x00 | **FAIL** |
| WiFi: scan | `scan sin INIT_COMPLETE`, error E/S | **FAIL** |
| Ethernet | rtl8169 DOWN | Sin DHCP |

## Hallazgos confirmados

### WiFi-1. NVM_ACCESS_CMD 0x88 no es comando del grupo 12

**soso (run4):** [`iwl_mvm_nvm.c`](lxdde/ports/iwlwifi/iwl_mvm_nvm.c) enviaba
`WIDE_ID(REGULATORY_AND_NVM_GROUP, 0x88)`.

**Linux:** `NVM_ACCESS_CMD = 0x88` es **LEGACY_GROUP**
([`commands.h:345`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/commands.h)).
Init unificado (`iwl_run_unified_mvm_ucode`, [`mvm/fw.c:613-653`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c))
solo manda `NVM_ACCESS_COMPLETE` (grp=12 id=0) y tras INIT_COMPLETE `NVM_GET_INFO`
(id=0x02) + MAC desde CSR.

**Corrección aplicada:** init sin `NVM_ACCESS_CMD`; `NVM_ACCESS_COMPLETE` +
`iwl_mvm_nvm_get_info_mac()` (NVM_GET_INFO + CSR strap/OTP).

### GPU-1. Heap WPR Ampere usaba parámetros GB20x

**soso (run4):** heap **134 MiB** (`WPR_BASE_RM_SIZE` 14 MiB + pmu 24 MiB en meta).

**Linux:** `r570_rm_ga102` → `BASE_RM_SIZE_TU10X` **8 MiB**, `pmuReservedSize=0`
([`rm/r570/rm.c:78-80`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r570/rm.c),
[`tu102.c:213-257`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/tu102.c)).

**Corrección aplicada:** `gsp_wpr_heap_size_ampere()` (8 MiB TU10X), meta Ampere
sin PMU; dumps post-booter ampliados (GSP mbox/os, heap).

### GPU-2. RPC tras booter (run3, ya corregido en 4a40cbca7)

Run4 **sí** llega a `run_gsp_rm_chain()` (`— sigue RPC` en log). Fallo actual:
`gsp_rpc_start` exige RISC-V activo (bit 7 en 0x111388), coherente con Linux
[`r535_gsp_init:1786-1789`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/gsp.c).

## Orden de corrección (estado)

| # | Cambio | Estado |
|---|---|---|
| 1 | Init WiFi sin NVM_ACCESS_CMD grp12; NVM_ACCESS_COMPLETE + NVM_GET_INFO | **Implementado** |
| 2 | Heap Ampere TU10X 8 MiB, pmuReservedSize=0 | **Implementado** |
| 3 | Dumps post-booter GSP (mbox, os, heap) | **Implementado** |
| 4 | Validar en placa: `INIT_COMPLETE`, `GSP_INIT_DONE`, `wifi scan` | Pendiente reflashear |

Reflashear (usuario):

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only kernel
```

## Qué no se ha hecho

- No se ha reflasheado el USB ni ejecutado ciclo en placa con el código nuevo.
- Hostchecks verdes no sustituyen validación TX/RPC/scan en silicio.
