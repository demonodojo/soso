# GB205 + AX211 — PNVM doorbell y CE COPY0 (15 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat, extraíble) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-15/` (ESP desmontada). Sin PSK en el informe.

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`dfec53f84-dirty`)** |
| Flush | **#35 @ 248486 ms** |
| Hardware | `10de:2f18` GB205 + `8086:7f70` AX211 + `10ec:8125` RTL8125 sin driver |
| Userspace | **`sosh —`** OK (pid=2) |
| GPU | `GSP_INIT_DONE res=0x0`; CE motor 10 semáforo=0; `pool VRAM=no` |
| WiFi | `UCODE_ALIVE_NTFY` real; timeout `INIT_EXTENDED_CFG` grp=2 id=0x03 |
| `ask hola` | mistral-7b en **CPU** (~44 s/capa); GPU sin cómputo usable |
| Halt | `UNLOADING_GUEST_DRIVER ok`, GSP-RM apagado |

Árboles Linux (solo lectura):

| Árbol | Tag / commit | Uso |
|---|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** | iwlwifi (`mvm/fw.c`, `fw/pnvm.c`, `iwl-prph.h`) |
| [`lxdde/reference/linux-master-nouveau/`](../lxdde/reference/linux-master-nouveau/) | **fc02acf** | CE r570 (`nvkm/.../rm/r535/ce.c`) |

Hostchecks (no validan silicio ni esta comparación):
`./scripts/l6-iwl-fw-hostcheck.sh` y `./scripts/l6-g3-gsp-hostcheck.sh` **OK**.
No cubren doorbell PNVM, DMA Falcon, scan real ni blit CE en placa.

Copias de la mañana (ROG AX200 flush #27) en
`target/usb-diagnostic-2026-09-15/prev-flush27/`. Ese hardware no es este USB.

## Tabla de etapas (flush #35)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (dfec53f84-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#35** @ 248 s | OK |
| GSP | `RPC fn=0x1001 (GSP_INIT_DONE) res=0x0` | **OK** |
| CE | `CE bring-up motor 10` → `semáforo no llegó a 1 … vale 0` | **FAIL** |
| VRAM pool | `GSP=booted, pool VRAM=no` | **FAIL** |
| Compute RM | `RM_ALLOC cls=0xcec0 … ok` | OK (objeto); sin CE no hay blit |
| `ask hola` | `mistral-7b en CPU (GPU sin cómputo usable)` | **FAIL** cómputo GPU |
| WiFi ALIVE | `firmware ALIVE (UCODE_ALIVE_NTFY)`; RX `grp=0 id=0x01` | **OK** |
| WiFi init | `timeout cmd grp=2 id=0x03` → `INIT_EXTENDED_CFG falló` (×4) | **FAIL** |
| Scan | `wifi scan:` sin BSS; cola bloqueada | **FAIL** |
| RTL8125 | `RED SIN DRIVER 83:00.0 10ec:8125` | etapa ausente |
| Halt | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` | **OK** |

Secuencia WiFi (primera pasada; se repite en 3 recuperaciones):

```
pnvm 13668 bytes (fichero 55208) → context-info gen3
→ UCODE_ALIVE_NTFY (RX id=0x01) + notifs 0xb1 / grp=2 id=0xff
→ send_cmd qid=0 doorbell=0x00000001 grp=2 id=0x03 (INIT_EXTENDED_CFG)
→ RX DEBUG_LOG 0xf7 (sin emparejar)
→ timeout slot=0; MVM parado
```

El firmware **sigue contestando por RX** tras ALIVE. No es timeout ALIVE ni
«ALIVE degradado».

## Hallazgos

### WIFI-1. Falta doorbell PNVM tras ALIVE (confirmado vs Linux 6.6)

**Síntoma:** primer HCMD `INIT_EXTENDED_CFG` (SYSTEM_GROUP 2, id 0x03, 12 B)
timeout. RX entrega `DEBUG_LOG_MSG` 0xf7; el FW no cierra el comando.

**soso:** [`iwl_ax211.c:164-167`](../lxdde/ports/iwlwifi/iwl_ax211.c) llama
`iwl_mvm_run_init` en cuanto hay ALIVE.
[`iwl_mvm_init.c:22-27`](../lxdde/ports/iwlwifi/iwl_mvm_init.c) manda
`INIT_EXTENDED_CFG` enseguida. El PNVM se copia a DMA y se escribe en
`scratch->ctrl_cfg.pnvm_cfg` **antes** de arrancar
([`iwl_trans.c:958-969`](../lxdde/ports/iwlwifi/iwl_trans.c)).
No existe `UREG_DOORBELL_TO_ISR6` ni espera de `PNVM_INIT_COMPLETE_NTFY`.

**Linux 6.6** [`mvm/fw.c:436-444`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c):
tras ALIVE válido, `iwl_pnvm_load` **antes** de `iwl_trans_fw_alive`.
[`fw.c:616-619`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)
manda `INIT_EXTENDED_CFG` **después**.
[`fw/pnvm.c:374-398`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/pnvm.c):

1. Parsea `sku_id[3]` del ALIVE v5 ([`fw.c:185-187`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)).
2. `iwl_pnvm_load_pnvm_to_trans` (casa SKU y `set_pnvm`).
3. Espera `WIDE_ID(REGULATORY_AND_NVM_GROUP=0xc, PNVM_INIT_COMPLETE_NTFY=0xFE)`.
4. `iwl_write_umac_prph(UREG_DOORBELL_TO_ISR6=0xA05C04, BIT(20))`
   ([`iwl-prph.h:465-470`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-prph.h)).

**Compatible con el síntoma:** sí. El FW AX211 (gen3, `iwlwifi-so-a0-gf-a0-89`)
tiene PNVM en scratch pero nadie pisa el doorbell; el primer HCMD de init
queda sordo.

**Descartado como causa de este timeout:** first_tb/TB0 (el 12 B cabe;
`send_cmd` se emite). RX gen3 (ALIVE + 0xf7 llegan). Cabecera wide SCD
(no se alcanza).

### GPU-1. CE en motor 10 (CE1) sin fallback a COPY0 (confirmado)

**Síntoma:** `CE bring-up motor 10` → semáforo=0 @ 2000 ms →
`CE selftest — la copia sysmem → VRAM no señalizó` → `pool VRAM=no`.
`g_ce_verified` queda 0; `ask` cae a CPU.

**soso:** [`gsp_top.c:158-176`](../lxdde/ports/nouveau/gsp_top.c)
`gsp_top_pick_ce_engine` elige el **primer** CE con runlist ≠ GR0
→ motor 10 (`NV2080_ENGINE_TYPE_COPY0 + 1`).
[`gsp_bringup.c:784-808`](../lxdde/ports/nouveau/gsp_bringup.c) reintenta
COPY0 **solo si `gsp_ce_init` falla**, no si el selftest falla.
Tras selftest ≠ 0 ([~L857-864](../lxdde/ports/nouveau/gsp_bringup.c))
sigue con `return 0` y compute ALLOC. `ce_rebind_engine` ([L923-927](../lxdde/ports/nouveau/gsp_bringup.c))
exige `g_ce_verified` ya a 1: no recupera el primer fallo.

**Linux** [`r535/ce.c:38`](../lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/ce.c):
`engineType = NV2080_ENGINE_TYPE_COPY0 + inst`. El camino habitual es
**inst=0 (COPY0)**. RM acepta COPY0 en este chip (`COPY0 (9) SÍ está en la lista`).
El GO de julio (VFIO GB205) usó COPY0.

**Compatible:** sí. CE1 no señaliza; no hay segundo intento en COPY0;
el pool VRAM está gated a `g_ce_verified`.

**Hipótesis (no primer fix):** dump RAMFC `USERD/GPFIFO NO_COINCIDE` y
`INTR_0=badf5040` — artefacto PRI conocido en lecturas GB205; no prueba
por sí solo un offset malo. `INVALID_CLASS 0x22` en golden GR (luego
`0xcec0` OK) no bloquea el pool.

### ETH-1. RTL8125 sin port (etapa ausente)

`10ec:8125` `RED SIN DRIVER`. No es un bug de una línea; hace falta port
o fit-drivers. No entra en el orden de esta placa (WiFi/GPU primero).

## Orden de corrección

### 1. Doorbell PNVM + espera 0xFE tras ALIVE (bloqueante WiFi)

- Archivos: [`iwl_mvm_init.c`](../lxdde/ports/iwlwifi/iwl_mvm_init.c),
  [`iwl_internal.h`](../lxdde/ports/iwlwifi/iwl_internal.h)
  (`UREG_DOORBELL_TO_ISR6`, `PNVM_INIT_COMPLETE_NTFY`),
  [`iwl_trans.c`](../lxdde/ports/iwlwifi/iwl_trans.c) (`iwl_write_prph` +
  `iwl_umac_prph`, mismo camino que `UREG_CPU_INIT_RUN`),
  parse `sku_id` del ALIVE v5 como Linux `fw.c:185-187`.
- Discrepancia: Linux `iwl_pnvm_load` (`pnvm.c:374-398`) **antes** de
  `INIT_EXTENDED_CFG`; soso salta el kick.
- Host: test que, con ALIVE mock y SKU ≠ 0, escribe `0xA05C04` BIT20 y
  no manda `INIT_EXTENDED_CFG` hasta `grp=0xc id=0xFE` (o timeout acotado).
- Placa: SOSOLOG con notif 0xFE / «PNVM complete», luego
  `INIT_EXTENDED_CFG` sin timeout, `init NVM listo`, scan `count>0`.

### 2. COPY0 primero o reintento COPY0 si el selftest de CE1 falla (bloqueante GPU)

- Archivos: [`gsp_top.c`](../lxdde/ports/nouveau/gsp_top.c)
  (`gsp_top_pick_ce_engine` → preferir inst 0 si COPY0 está en la lista RM),
  [`gsp_bringup.c`](../lxdde/ports/nouveau/gsp_bringup.c)
  (`run_chan_ce_stage`: si selftest ≠ 0 y el motor ≠ COPY0, fini + COPY0 +
  selftest; no `return 0` con `g_ce_verified=0` fingiendo éxito).
- Discrepancia: Linux r535 arranca COPY0+inst; soso elige CE1 por PTOP y
  no cae a COPY0 cuando el blit no señaliza.
- Host: hostcheck GSP que el fallback COPY0 se invoca si el primer selftest
  falla (mock); no sustituye silicio.
- Placa: `CE readback verificado`, `pool VRAM=sí`, `ask` no forzado a CPU
  solo por CE.

### 3. Registro (esta ejecución)

- `docs/hw-matrix.json`: `gb205-dgpu` y `ax211-wifi` con etapas fail/ok
  de flush #35; `arranques_consecutivos_ok` = 0; notas → este fichero.
- Este informe.

## Flush #42 (tarde, mismo USB)

Copias en `target/usb-diagnostic-2026-09-15-run2/`. Kernel **0.2.2
(`dfec53f84-dirty`)**, flush **#42 @ ~200 s**, `sosh —` pid=2.

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| GSP | `GSP_INIT_DONE res=0x0` | **OK** |
| CE | `CE readback verificado (G4e GO)` | **OK** (COPY0 validado) |
| VRAM pool | `pool VRAM=sí` | **OK** |
| RM_ALLOC | `RM_ALLOC cls=0xcec0 ok` | OK |
| ask | `:tiny hola` → backend GPU, prefetch VRAM; **SIG130** antes de inferir | **pendiente** cómputo |
| WiFi ALIVE | `sku=0x510d1 0x0 0x0` | **OK** |
| PNVM | `pnvm 13668 bytes` → doorbell `prph=0xd05c04 val=0x100000` | **FAIL** (timeout `PNVM_INIT_COMPLETE_NTFY`) |
| init MVM | `init MVM incompleto` | **FAIL** |
| scan | `scan sin INIT_COMPLETE` | **FAIL** |

Secuencia WiFi (causa raíz confirmada vs Linux 6.6):

```
pnvm 13668 B (primera sección SKU 0x610d1, no 0x510d1 del ALIVE)
→ doorbell PNVM BIT(20) @ 0xa05c04+0x300000
→ timeout PNVM_INIT_COMPLETE_NTFY (0xFE)
→ init MVM incompleto → scan sin INIT_COMPLETE
```

**Fix implementado (kernel, sin reflash aún):** parsear `.pnvm` por SKU tras ALIVE;
`pnvm_cfg` vacío en `start_fw`; publicar descriptor fragmentado (capa 32) o
concat de 2 chunks; doorbell + espera 0xFE. Hostcheck:
`./scripts/l6-iwl-fw-hostcheck.sh` — SKU `0x510d1` → **13716 B**, descriptor OK.

Tras reflash: `cargo xtask flash-usb-live /dev/sda --yes --only kernel`.

## Qué no se ha hecho

- USB **no reflasheado** con el fix PNVM (pendiente usuario).
- Hostcheck verde no acredita PNVM en silicio hasta placa.
- No `record-boot` / `--boot-ok`.
- RTL8125 y assoc/DHCP WiFi quedan fuera hasta que INIT + scan pasen.
