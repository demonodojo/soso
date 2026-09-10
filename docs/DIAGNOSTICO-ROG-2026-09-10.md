# ROG: GPU GA107 y WiFi AX200 — diagnóstico (10 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl`. Copia:
`target/usb-diagnostic-2026-09-10/` (SOSOWIFI vacío; sin PSK). ESP desmontada al terminar.

| Campo | run13 (este USB) | run12 |
|---|---|---|
| Kernel USB | **0.2.2 (1df51e817-dirty)** | igual (sin commit; sí reflasheado) |
| Flush / uptime | **#33, 475 s** | #43, 1024 s |
| Hardware | 10de:249c + 8086:2723 + 10ec:8168 | igual |
| sosh | **SÍ** (+ `wifi scan` + `ask hola`) | SÍ (+ halt) |
| GSP | **GSP_INIT_DONE** + vaspace gp100 | igual |
| Canal COPY0 | **`RM_ALLOC 0xc56f chid=1` ok + CE G4e GO** | NO_MEMORY chid=0 |
| GR0 / pool | **RPC muerto** en `0x20802a08`; `pool VRAM=no` | no alcanzado (sin canal) |
| WiFi | ALIVE + SF ok; **timeout TX_ANT 0x98** | TX_ANT/PHY/MCC/SCAN_CFG timeout |
| fatlog | sí | sí |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/fw.c`, `mvm/sf.c`, `pcie/tx-gen2.c`, `iwl-csr.h`, `iwl-nvm-parse.c` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** — `r535/fifo.c`, `r535/bar.c`, `r570/fifo.c` |

Hostchecks (host, sin silicio): `l6-iwl-fw-hostcheck.sh` OK (wide/scan/contrato/TLV),
`l6-g3-gsp-hostcheck.sh` OK (incluye segundo canal USERD chid=2).
**No cubren** transporte Ampere post-CE, walk BAR1 en placa, ni TX_ANT en el AX200.

## Tabla de etapas (run13)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `wifi scan` + `ask hola` (Qwen3.8-27B en CPU) | **OK** |
| fatlog | flush **#33** @ 475 s | OK |
| lxdde pci / GSP | `BAR0 boot0=0xb74000a1`, `GSP_INIT_DONE`, PRAMIN `0x001700` | **OK** |
| Canal COPY0 / CE | `RM_ALLOC cls=0xc56f chid=1` ok; `CE selftest OK`; `CE readback verificado (G4e GO)` | **OK** (nuevo vs run12) |
| BAR1 | PDB instancia `0x31b233f89e94b000`; PD3=`0xbad0fb2f…`; «las tablas SÍ se escriben» | **FAIL** |
| Canal GR0 | `RM_CONTROL cmd=0x20802a08` → `no llegó fn=0x004c (0 mensajes)` | **FAIL** |
| VRAM pool | `pool VRAM=no`; `G6` no inicializado | **FAIL** |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, `SHARED_MEM_CFG ok`, `SF_INIT_OFF` | **OK** |
| WiFi up | timeout `grp=0 id=0x98` (`TX_ANT`); `abort up`; scan E/S | **FAIL** |
| Ethernet | rtl8169 `enlace DOWN`; `net: dhcp…` sin lease | sin DHCP (no marcar ok) |

**Interpretación:** el reflasheo post-run12 **cierra el NO_MEMORY de GPFIFO** (`chid=1` = `rsvd_chids`).
G4e Ampere es GO. El bloqueo GPU pasa a: (1) parche BAR1 con PDB basura **después** del CE, que deja
el RPC mudo; (2) `gsp_buf_init` no se llama si GR0 falla, aunque el CE ya mueve VRAM.
WiFi: HCMD wide está en el USB; TX_ANT sigue sordo tras SF (Linux manda SF `CMD_ASYNC`).

## Hallazgos

### GPU-1. Parche BAR1 con PDB PRI-error mata el RPC — **confirmado**

**Síntoma:** tras `CE readback verificado (G4e GO)` el log camina BAR1 con
`PDB=0x31b233f89e94b000`, lee `0xbad0fb2fbad0fb2e`, escribe tablas, reintenta `bar2Pde`.
El siguiente `RM_CONTROL` (`CE_GET_FAULT_METHOD_BUFFER_SIZE` para GR0) no recibe ningún mensaje.

**soso:** [`gsp_bringup.c`](lxdde/ports/nouveau/gsp_bringup.c) (tras G4e, ~L656–758) llama
`gsp_bar1_inst_probe` / `gsp_bar1_selftest` / escribe PTEs ([`gsp_bar1.c`](lxdde/ports/nouveau/gsp_bar1.c)
`gsp_pramin_wr32`, selftest escribe 4096 B por la apertura).

**Linux:** `r535_bar_bar1_init` **no construye** tablas: envuelve `gsp->bar.rm_bar1_pdb`
([`r535/bar.c:128–148`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/bar.c)).
BAR2 usa `UPDATE_BAR_PDE` RPC (`r535_bar_bar2_update_pde`). No hay walk de bloque de instancia
ni writes a un PDB `0xbad0…` después del CE.

**Compatible con el síntoma:** sí. RPC vivo hasta el parche; 0 mensajes después.

### GPU-2. `gsp_buf_init` no corre si GR0 falla — **confirmado**

**Síntoma:** `CE readback verificado` y `pool VRAM=no` en el mismo arranque.
`gpu: … GSP=booted, pool VRAM=no`. `ask` cae a CPU.

**soso:** [`run_compute_stage`](lxdde/ports/nouveau/gsp_bringup.c) L772–780 hace `return -1` si
`gsp_chan_init(… GR0)` falla. `gsp_buf_init` está **después** (L855) y nunca se ejecuta.
El pool solo necesita VRAM + VMM + CE, que ya están.

**Linux:** el pool/FB no está atado al canal GR; `r535_fifo_ctor` pide el method buffer **una vez**
al crear fifo ([`r535/fifo.c:565–575`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/fifo.c)),
no por canal, y el CE no depende de GR0.

**Compatible:** sí. Con CE GO el pool debería montarse aunque GR0 falle.

### GPU-3. Method buffer por canal (segunda query) — **confirmado (agrava GR0)**

**soso:** [`chan_query_mthdbuf_size`](lxdde/ports/nouveau/gsp_chan.c) L55–81 / L362 pregunta
`NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE` (`0x20802a08`) **en cada** `gsp_chan_init`.
La primera (COPY0) funciona (`mthdbuf=20480 B`). La segunda (GR0) es el primer RPC tras BAR1 y timeout.

**Linux:** una query en `r535_fifo_ctor`, resultado en `fifo->rm.mthdbuf_size` para todos los canales.

### GPU-4. `chid = idx + rsvd_chids(1)` — **validado en placa**

run12: `NO_MEMORY chid=0`. run13: `chid=1 flags=0x00200120` ok.
Linux [`r570/fifo.c:212`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r570/fifo.c)
`.rsvd_chids = 1`. No reabrir.

### WiFi-1. SF síncrono vs Linux `CMD_ASYNC`; TX_ANT timeout — **confirmado (SF); hipótesis causal TX_ANT**

**Síntoma:** `SHARED_MEM_CFG ok` → `Smart Fifo SF_INIT_OFF` (sin línea `iwl_rx` de `0xd1`) →
`timeout cmd grp=0 id=0x98` → `TX ant no configurada — abort up`. Reintento de scan: también
timeout `SHARED_MEM_CFG`. MAC anunciada `80:00:00:01:00:01`.

**soso:** [`iwl_mvm_up_minimal`](lxdde/ports/iwlwifi/iwl_mvm_up.c) L230–236 espera SF
(`send_cmd_wait`, 2000 ms) y luego TX_ANT síncrono ([`iwl_mvm_nvm.c:209–223`](lxdde/ports/iwlwifi/iwl_mvm_nvm.c)).
Cabecera wide gen2 en [`iwl_trans.c:747–758`](lxdde/ports/iwlwifi/iwl_trans.c) (hostcheck verde).

**Linux:** `iwl_mvm_sf_update` manda `REPLY_SF_CFG_CMD` con **`CMD_ASYNC`**
([`mvm/sf.c:211`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/sf.c)). Luego
`iwl_send_tx_ant_cfg` síncrono ([`mvm/fw.c:1530–1546`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)).
`TX_ANT_CONFIGURATION_CMD = 0x98` payload 4 B `valid` le32. Versión en el `cmd.id` (0), no lookup TLV
en `send_cmd_pdu`.

**Compatible:** SF async vs wait es discrepancia real. Que el wait de SF «ok» sin RX de `0xd1` y el
TX_ANT timeout encajan con completar SF en falso / mandar 0x98 mientras el FW aún procesa SF.
No se puede atribuir el timeout **solo** a la cabecera de 4 B (ya no está en este USB).

### WiFi-2. MAC `80:00:00:01:00:01` — **hipótesis**

Offsets CSR 0x380 OTP/STRAP coinciden con Linux [`iwl-csr.h:640–644`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-csr.h).
El valor pasa `iwl_mac_valid_unicast` (unicast, no cero) y no se descarta. No es la causa del
timeout de 0x98; conviene loguear STRAP vs OTP crudos y no anunciar «MAC NVM» si salió del CSR.

### Otros (no bloquean el plan de drivers)

- `init: sosh viva sin /tmp/sosh-ready; no confirmo OTA` — C7 live, no es GPU/WiFi.
- rtl8169 DOWN — cable/enlace; no mezclar con DHCP WiFi.
- `hw-inv: no pude escribir /etc/soso-hw (Io)` — inventario; hipótesis.

## Orden de corrección

| # | Cambio | Archivos | Discrepancia Linux | Hecho host / placa |
|---|---|---|---|---|
| 1 | No escribir PTEs BAR1 si el PDB es PRI-error (`0xbad0…`) o ≠ `bar1PdeBase`; no selftest que parche tras CE | `gsp_bar1.c`, `gsp_bringup.c` | `r535_bar_bar1_init` solo envuelve `rm_bar1_pdb` | Hostcheck: PDB `0xbad0` ⇒ 0 writes. Placa: RPC vivo **después** de CE (siguiente `RM_CONTROL` con respuesta) |
| 2 | Montar pool VRAM con CE solo: `gsp_buf_init` tras G4e, **antes** de BAR1/GR0; no `return -1` que lo salte | `gsp_bringup.c` | pool/FB no depende de GR0 | Hostcheck: `gsp_buf_init` se llama con CE y sin canal GR. Placa: `pool VRAM=sí` con o sin GR0 |
| 3 | Cachear `GET_FAULT_METHOD_BUFFER_SIZE` una vez (fifo); GR0 reutiliza tamaño; chid=2 | `gsp_chan.c` | `r535_fifo_ctor` L565–575 | Hostcheck ya tiene 2.º canal. Placa: `RM_ALLOC` GR0 ok, no timeout `0x20802a08` |
| 4 | SF `CMD_ASYNC` (no esperar `0xd1`); luego TX_ANT sync; loguear ver/len del wide 0x98; no abortar up con un ACK inventado | `iwl_mvm_up.c`, `iwl_trans.c` | `sf.c:211` async; `fw.c:1544` TX_ANT sync | Hostcheck: SF no cierra pending; TX_ANT 8 B header + 4 B payload. Placa: `TX_ANT_CONFIGURATION ok` → DQA/PHY/MCC/SCAN_CFG → `wifi scan` con fin normal |

## Qué no se ha hecho

- No se ha tocado código de drivers en esta lectura.
- No se ha reflasheado. No `record-boot` / `--boot-ok` (`arranques_consecutivos_ok` se dejó en 0).
- Hostchecks verdes no sustituyen el RPC post-CE ni TX_ANT en silicio.
- No se ha validado `ask` con pool VRAM ni asociación WPA2.
