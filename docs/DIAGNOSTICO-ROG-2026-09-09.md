# ROG: GPU GA107 y WiFi AX200 — diagnóstico (9 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-09-run{3,4,5}/`. ESP desmontada al terminar.

| Campo | run4 | run5 (último USB) |
|---|---|---|
| Kernel USB | 0.2.2 (4a40cbca7) | **0.2.2 (4a40cbca7-dirty)** |
| Flush / uptime | #8, 280 s | **#13, 297 s** |
| Hardware | 10de:249c + 8086:2723 + 10ec:8168 | igual |
| sosh | OK | OK |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | 6.6.32 — `mvm/fw.c`, `fw/api/nvm-reg.h`, `fw/api/commands.h` |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf` — `tu102.c`, `rm/r570/rm.c`, `rm/r535/gsp.c` |

Hostchecks tras correcciones run5: `l6-iwl-fw-hostcheck.sh` OK,
`l6-g3-gsp-hostcheck.sh` OK (incluye `gsp_cpu_seq` / CORE_RESUME).

## Tabla de etapas (run5)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh — escribe 'help'` | OK |
| GPU: FWSEC-FRTS + booter | `booter_load boot ok`, WPR2 LO=`0x3f4000000`, heap **128 MiB** | OK (Falcon) |
| GPU: GSP-RM / RPC | 4 s `RISC-V aún inactivo (cpuctl=0x10)`, `gsp_rpc_start` corta sin `RPC fn=` | **FAIL** |
| GPU: pool VRAM | `GSP=fallo`, `pool VRAM=no` | **FAIL** |
| WiFi: ALIVE | `UCODE_ALIVE_NTFY` | OK |
| WiFi: INIT_EXTENDED_CFG | rsp grp=2 id=0x03 | OK |
| WiFi: INIT_COMPLETE | `INIT_COMPLETE_NOTIF` y acto seguido `timeout INIT_COMPLETE_NOTIF` | **FAIL** (carrera) |
| WiFi: scan | `scan sin INIT_COMPLETE`, error E/S | **FAIL** |
| Ethernet | rtl8169 DOWN | Sin DHCP |

## Hallazgos confirmados (run5)

### WiFi-1. Carrera INIT_COMPLETE vs NVM_ACCESS_COMPLETE

**soso (run5):** [`iwl_mvm_init.c`](lxdde/ports/iwlwifi/iwl_mvm_init.c) ponía
`init_complete = 0` **después** de `send_cmd_wait(NVM_ACCESS_COMPLETE)`, que ya
drena RX y puede marcar el flag (`LEGACY` `0x4` = INIT_COMPLETE_NOTIF).

**Linux:** `iwl_init_notification_wait(..., INIT_COMPLETE_NOTIF)` **antes** de
cargar ucode y de `NVM_ACCESS_COMPLETE`; luego `iwl_wait_notification`
([`mvm/fw.c:595-664`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)).

**Corrección aplicada (d91bc18f9):** `init_complete = 0` antes de
`INIT_EXTENDED_CFG` / `NVM_ACCESS_COMPLETE`; no volver a borrarlo. Si el flag ya
está al salir del wait, no esperar de nuevo.

### WiFi-2. NVM_ACCESS_CMD 0x88 (run4, ya en USB run5)

Init unificado sin `NVM_ACCESS_CMD` grp12; `NVM_ACCESS_COMPLETE` +
`NVM_GET_INFO` + MAC desde CSR ([`iwl_mvm_nvm.c`](lxdde/ports/iwlwifi/iwl_mvm_nvm.c)).

### GPU-1. Heap WPR Ampere (run4/run5, ya en USB run5)

Heap **128 MiB** (TU10X 8 MiB + pmu=0). Confirmado en run5 SOSOLOG.

### GPU-2. RPC tras booter aborta si cpuctl bit7=0

**soso (run5):** [`gsp_bringup.c`](lxdde/ports/nouveau/gsp_bringup.c) esperaba
4 s a RISC-V activo; [`gsp_rpc_start`](lxdde/ports/nouveau/gsp_rpc.c) salía sin
leer msgq si `ga102_flcn_riscv_active` fallaba. GSP mbox=`0x80000000` (cola sin
drenar). Si el firmware pide `CORE_RESET`/`CORE_RESUME` en esa ventana, RISC-V
queda en halt (`cpuctl=0x10`).

**Linux:** `tu102_gsp_init` → `booter_load` → enseguida `r535_gsp_init`:
escribe `app_version`, poll `GSP_INIT_DONE` que despacha `GSP_RUN_CPU_SEQUENCER`
([`tu102.c:204-209`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/tu102.c),
[`r535/gsp.c:1786-1791`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/gsp.c)).

**Corrección aplicada:** quitar espera 4 s como juez; tras booter escribir
`app_version` y entrar en poll RPC/msgq aunque bit7=0; dumps ampliados
(`0x1180f8`, msgq wptr, app_version).

### GPU-3. CORE_RESUME del sequencer (no-op en run5)

**soso (run5):** [`gsp_cpu_seq.c`](lxdde/ports/nouveau/gsp_cpu_seq.c) tenía
`CORE_RESUME` como no-op.

**Linux:** [`r535/gsp.c:1092-1122`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/gsp.c):
`ga102_gsp_reset` → libos mbox → `nvkm_falcon_start(SEC2)` → poll
`0x1180f8 & 0x04000000` → SEC2 mbox0==0 → `app_version` → `riscv_active`.

**Corrección aplicada:** `seq_core_resume()` + `falcon_lx_start()` en SEC2
`0x840000`; `gsp_cpu_seq_set_ctx()` desde `run_gsp_rm_chain()`.

## Orden de corrección (estado)

| # | Cambio | Estado |
|---|---|---|
| 1 | Init WiFi sin NVM_ACCESS_CMD grp12; NVM_ACCESS_COMPLETE + NVM_GET_INFO | **En USB run5** |
| 2 | Heap Ampere TU10X 8 MiB, pmuReservedSize=0 | **En USB run5** |
| 3 | INIT_COMPLETE: armar flag antes de NVM_ACCESS_COMPLETE | **Implementado (d91bc18f9)** |
| 4 | RPC/msgq tras booter sin abortar por cpuctl; app_version inmediato | **Implementado (d91bc18f9)** |
| 5 | CORE_RESUME sequencer (reset RISC-V, libos, SEC2, 0x1180f8) | **Implementado (d91bc18f9)** |
| 6 | Validar en placa: `INIT_COMPLETE`, `GSP_INIT_DONE`, `wifi scan`, `pool VRAM=sí` | Pendiente reflashear |

Reflashear (usuario):

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only kernel
```

## Qué no se ha hecho

- No se ha reflasheado el USB con d91bc18f9 ni ejecutado ciclo en placa con las
  correcciones INIT_COMPLETE / RPC / CORE_RESUME.
- Hostchecks verdes no sustituyen validación TX/RPC/scan en silicio.
- No se ha ejecutado `record-boot` ni `--boot-ok`.
