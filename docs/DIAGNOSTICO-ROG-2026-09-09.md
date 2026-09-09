# ROG: GPU GA107 y WiFi AX200 — ejecución run3 (9 sep 2026)

## Evidencia y alcance

Lectura de `/dev/sda1` (ESP `kernel`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-09-run3/`. ESP desmontada al terminar.

| Campo | Valor |
|---|---|
| Dispositivo ESP | `/dev/sda1` (500G USB, kernel-x86_64 31 MiB empaquetado) |
| Kernel USB | **0.2.2 (669afd2c1-dirty)** |
| Flush / uptime | flush #10, uptime 363744 ms |
| Hardware | GPU **10de:249c (GA107)** + WiFi **8086:2723 (AX200)** + rtl8169 **10ec:8168** |
| BOOTMARK | UEFI alcanzado, bootsoso.efi 156672 B |

Árboles Linux usados (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | 6.6.32 — iwlwifi `mvm/fw.c`, `mvm/nvm.c`, `fw/api/nvm-reg.h` |
| `lxdde/reference/linux-master-nouveau/` | commit `fc02acf` — `nvkm/subdev/gsp/{tu102,ga102}.c`, `nvkm/falcon/ga102.c` |

Hostchecks ejecutados (salida en run3): `l6-iwl-fw-hostcheck.sh` (OK),
`l6-g3-gsp-hostcheck.sh` (OK). No cubren transporte iwl post-ALIVE ni la
secuencia Ampere booter→RISC-V en placa.

Esta ejecución **no** valida GB205 ni AX211. No se han modificado drivers.

## Tabla de etapas

| Etapa | Evidencia SOSOLOG (run3) | Resultado |
|---|---|---|
| Arranque UEFI | BOOTMARK: shim + bootsoso.efi | OK |
| Userspace | L407: `sosh — escribe 'help'` | OK |
| GPU: carga firmware | L276–288: 4 blobs ga102, radix3 verificada | OK |
| GPU: FWSEC-FRTS | L309: `FWSEC-FRTS OK — WPR2 0x3ffe00000-0x3ffee0000` | OK |
| GPU: booter SEC2 | L316–324: base **0x840000**, sig patch, `booter_load boot ok` | OK (Falcon SEC2) |
| GPU: GSP-RM / RPC | L325–327: `RISC-V inactivo cpuctl=0x10`, `GSP=fallo` | **FAIL** |
| GPU: pool VRAM | L354: `pool VRAM=no` | **FAIL** |
| WiFi: ALIVE | L341: `UCODE_ALIVE_NTFY`, `alive=true` | OK |
| WiFi: INIT_EXTENDED_CFG | L343–344: qid=0 doorbell=0x1, rsp grp=2 id=0x03 | OK (progreso vs run2) |
| WiFi: NVM | L345–349: timeout grp=12 id=0x88 / 0x00 | **FAIL** |
| WiFi: INIT_COMPLETE | L350: `init MVM incompleto` | **FAIL** |
| WiFi: scan | L411–412: `scan sin INIT_COMPLETE`, `error de E/S` | **FAIL** |
| Ethernet | L362: rtl8169 enlace DOWN 10M half | Sin DHCP |

## Hallazgos

### GPU-1. Booter SEC2 arranca pero el port aborta antes de RPC

**Síntoma:** L325–327: booter falcon ok, luego `Ampere booter ok pero RISC-V
inactivo (cpuctl=0x00000010 … bcr@0x1668=0x111)` y `GSP=fallo`.

**soso:** [`lxdde/ports/nouveau/gsp_bringup.c:414-432`](lxdde/ports/nouveau/gsp_bringup.c)
— tras `falcon_lx_hsfw_boot_mbox` espera 4 s a que `NV_PRISCV_CPUCTL` (0x111388)
tenga bit 7 (`CPUCTL_ACTIVE_STAT`); con cpuctl=0x10 aborta y **no llama**
`run_gsp_rm_chain()`.

**Linux:** [`tu102.c:189-209`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/tu102.c)
— `tu102_gsp_init()` ejecuta `tu102_gsp_booter_load()` y pasa directamente a
`r535_gsp_init()` sin comprobar cpuctl bit 7 en ese punto. La actividad RISC-V
se valida vía RPC (`GSP_INIT_DONE`), no como puerta previa.

**Registro activo:** [`ga102.c:28-30`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/falcon/ga102.c)
— `ga102_flcn_riscv_active()` lee `addr2+0x388` bit 7 (= 0x111388 bit 7, mismo
registro). Valor observado **0x10** → bit 7 ausente.

**Clasificación:** **Confirmado** — el booter SEC2 completó (L324) pero el
check en soso impide la cadena RPC que Linux sí intenta. Posible causa raíz
adicional (booter no levantó GSP-RM) queda sin diagnosticar porque RPC nunca
arranca.

**Correcciones previas validadas en run3:** SEC2 override 0x840000 (L316),
staging DMA payload, parche firma HS (L318), reset RISC-V 0x111 (L315), sin
ACR previo al booter.

### GPU-2. PTOP SEC2 addr=0x087000 (informativo)

**Evidencia:** L263: `PTOP 1: SEC20 addr=0x087000`.

**soso/Linux:** PTOP decodifica bien; Linux [`ga102_sec2_new()`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/engine/sec2/ga102.c)
fuerza 0x840000 para MMIO. soso ya usa el override (L316). **Descartado**
como causa del fallo actual.

### WiFi-1. INIT_EXTENDED_CFG responde (progreso doorbell/cola)

**Evidencia:** L343–344: `send_cmd qid=0 doorbell=0x00000001`, respuesta
`grp=2 id=0x03 seq=0x0000 len=4`.

**soso:** [`iwl_trans.c:723-725`](lxdde/ports/iwlwifi/iwl_trans.c) — doorbell
`write_ptr | (qid << 16)`; cola HCMD=0.

**Linux:** [`queue/tx.c:69`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/queue/tx.c)
— mismo encoding.

**Clasificación:** **Confirmado OK** — mejora respecto a run2 (timeout
INIT_EXTENDED_CFG).

### WiFi-2. NVM_ACCESS_CMD con struct incorrecta

**Síntoma:** L345–349: timeout `grp=12 id=0x88` (NVM_ACCESS_CMD) y
`id=0x00` (NVM_ACCESS_COMPLETE).

**soso:** [`iwl_internal.h:491-495`](lxdde/ports/iwlwifi/iwl_internal.h) —
```c
struct iwl_nvm_access_cmd {
    uint16_t offset;
    uint16_t length;
    uint32_t type;   /* layout incorrecto */
};
```
[`iwl_mvm_nvm.c:32-35`](lxdde/ports/iwlwifi/iwl_mvm_nvm.c) — no setea
`op_code` ni `target`.

**Linux:** [`fw/api/nvm-reg.h:105-112`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/nvm-reg.h)
— API v2:
```c
struct iwl_nvm_access_cmd {
    u8 op_code;      /* NVM_READ_OPCODE */
    u8 target;
    __le16 type;
    __le16 offset;
    __le16 length;
};
```
[`mvm/nvm.c:76-80`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/nvm.c).

**Clasificación:** **Confirmado** — payload de 8 B mal formado; firmware ignora
o no responde. INIT_EXTENDED_CFG demuestra que el transporte TX/RX funciona.

### WiFi-3. Secuencia init incompleta → sin INIT_COMPLETE → scan bloqueado

**Síntoma:** L350–351, L369, L411–412.

**soso:** [`iwl_mvm_init.c:20-75`](lxdde/ports/iwlwifi/iwl_mvm_init.c) — tras
fallo NVM no llega a `INIT_COMPLETE_NOTIF` ni `radio_ready`.

**Linux:** [`mvm/fw.c:576-666`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)
— INIT_EXTENDED_CFG → (NVM opcional) → NVM_ACCESS_COMPLETE → PHY_CFG (gen3) →
espera INIT_COMPLETE.

**Clasificación:** **Confirmado** — efecto cascada de WiFi-2; scan UMAC v15 y
RX MQ siguen pendientes de validar tras INIT_COMPLETE.

## Orden de corrección

### 1. GPU: permitir cadena RPC tras booter (bloqueante)

**Cambio:** En `run_ampere_booter()`, eliminar o relajar la puerta
`CPUCTL_ACTIVE_STAT` (L414–432); continuar a `run_gsp_rm_chain()` como
`tu102_gsp_init()`. Mantener log diagnóstico de cpuctl/mbox/WPR2.

**Validar host:** `./scripts/l6-g3-gsp-hostcheck.sh` (sin regresión).

**Validar placa:** Log debe mostrar `GSP_INIT_DONE` o error RPC concreto;
criterio hecho: `GSP=…` distinto de `fallo` o mensaje RPC identificable.

### 2. WiFi: corregir `iwl_nvm_access_cmd` API v2 (bloqueante)

**Cambio:** Alinear struct con `nvm-reg.h` (op_code, target, type le16,
offset/length le16). En lectura MAC usar `NVM_READ_OPCODE`. Revisar si AX200
puede omitir NVM_ACCESS_CMD y pasar a NVM_ACCESS_COMPLETE como Linux cuando
no hay NVM externo.

**Validar host:** extender hostcheck o test unitario del struct (tamaño 8 B).

**Validar placa:** Log `NVM_ACCESS_CMD` con respuesta grp=12; luego
`INIT_COMPLETE_NOTIF`; `wifi scan` sin error E/S.

### 3. WiFi: completar init y scan (tras #2)

**Cambio:** Secuencia Linux en `iwl_mvm_run_init`: NVM_ACCESS_COMPLETE,
PHY_CFG omitido en gen2, espera INIT_COMPLETE, scan UMAC v15/v17.

**Validar placa:** `count>0` o `SCAN_COMPLETE_UMAC notif=1`; BSS visibles.

### 4. GPU: si RPC falla tras #1, diagnosticar booter→GSP-RM

**Cambio:** Comparar mbox0/mbox1, WPR meta phys, y contenido boot params con
Linux `tu102_gsp_booter_load()`. Registrar NOCAT si RPC arranca.

## Qué no se ha hecho

- No se han modificado drivers en esta sesión.
- No se ha reflasheado el USB.
- No se ha ejecutado ciclo de corrección en placa.
- Hostchecks verdes no sustituyen validación TX/RPC en silicio.
