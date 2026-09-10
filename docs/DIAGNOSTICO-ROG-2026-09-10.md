# ROG: GPU GA107 y WiFi AX200 — diagnóstico (10 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl`. Copias:

- run14: `target/usb-diagnostic-2026-09-10-run2/` (SOSOWIFI vacío; sin PSK)
- run13: `target/usb-diagnostic-2026-09-10/`

ESP desmontada al terminar.

| Campo | run14 (último) | run13 | run12 |
|---|---|---|---|
| Kernel USB | **0.2.2 (276404696)** | 0.2.2 (1df51e817-dirty) | igual (refl.) |
| Flush / uptime | **#41, 787 s** | #33, 475 s | #43, 1024 s |
| Hardware | 10de:249c + 8086:2723 + 10ec:8168 | igual | igual |
| sosh | **SÍ** (+ `wifi scan` + halt) | SÍ (+ `wifi scan` + `ask hola`) | SÍ (+ halt) |
| GSP / CE | **GSP_INIT_DONE**; CE G4e GO; **G6 buffers listos** | GSP ok; CE G4e GO | NO_MEMORY chid=0 |
| GR0 / pool | **GR0 chid=2 ok**; compute `0xc7c0` ok; **pool VRAM=no** (CE stuck SASS) | RPC muerto `0x20802a08`; pool=no | no alcanzado |
| WiFi | ALIVE + INIT; **NVM_GET_INFO falló**; scan E/S | SF ok; **timeout TX_ANT 0x98** | TX_ANT/PHY/MCC timeout |
| Apagado | **GSP-RM apagado … dma=off** | no registrado | — |
| fatlog | sí | sí | sí |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/fw.c`, `mvm/sf.c`, `pcie/tx-gen2.c`, `iwl-csr.h`, `iwl-nvm-parse.c` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** — `r535/fifo.c`, `r535/bar.c`, `r570/fifo.c` |

Hostchecks (host, sin silicio): `l6-iwl-fw-hostcheck.sh` OK (wide/scan/contrato/TLV + NVM v4 468 B),
`l6-g3-gsp-hostcheck.sh` OK (segundo canal USERD chid=2 + CE saxpy 512 B page-aligned + grctx map).
Tras fixes run14: hostchecks verdes; **validación en placa pendiente** (reflash kernel).

## Tabla de etapas (run14)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `wifi scan` + halt | **OK** |
| fatlog | flush **#41** @ 787 s | OK |
| lxdde pci / GSP | `GSP_INIT_DONE`, vaspace gp100 | **OK** |
| Canal COPY0 / CE G4e | `RM_ALLOC cls=0xc56f chid=1`; `CE selftest OK`; `CE readback verificado (G4e GO)` | **OK** |
| G6 / pool | `G6 — buffers VRAM listos`; luego `pool VRAM=no` (`ce.stuck`) | **FAIL** |
| SASS saxpy | `gsp_compute_stage_sass` 512 B → semáforo vale 2 (esperado 3) | **FAIL** |
| Canal GR0 | `RM_ALLOC 0xc56f chid=2` ok; compute `0xc7c0` ok | **OK** (vs run13) |
| grctx ATTRIBUTE_CB | mapeo `size=0x851200` (no múltiplo 4 KiB) → sin promocionar | **FAIL** |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE_NOTIF` | **OK** |
| WiFi NVM | `iwl_rx grp=12 id=0x02 len=468 st=0` → `NVM_GET_INFO falló` (sin timeout) | **FAIL** |
| WiFi scan | `scan sin INIT_COMPLETE` / E/S | **FAIL** |
| Apagado | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` | **OK** |

**Interpretación run14:** run13 fixes (BAR1 no escribe, G6, chid=2 GR0) **ya en el USB**.
Nuevos bloqueos: (1) falso rechazo FW en `len_n_flags & 0x40` para NVM v4 468 B;
(2) CE copia SASS 512 B sin encoding boa0b5 → `ce.stuck` aunque G4e fue GO;
(3) mapeo grctx con tamaño RM crudo 0x851200 sin alinear a página.

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

### WiFi-3. NVM_GET_INFO v4: `len_n_flags & 0x40` no es rechazo FW — **confirmado (run14)**

**Síntoma:** `iwl_rx: grp=12 id=0x02 seq=0x0002 len=468 st=0` seguido de `NVM_GET_INFO falló`
sin `timeout cmd`. `radio_ready` baja → init MVM incompleto → `scan sin INIT_COMPLETE`.

**soso:** [`iwl_trans.c`](lxdde/ports/iwlwifi/iwl_trans.c) interpretaba
`cmd_fw_err = (len_n_flags & IWL_CMD_FAILED_MSK)` con `IWL_CMD_FAILED_MSK = 0x40`.
Payload 468 B ⇒ `len = 472`; el bit 0x40 es parte del **tamaño** (bits 13:0), no un flag de error.

**Linux 6.6:** `IWL_CMD_FAILED_MSK` vive en **iwlegacy** (`hdr.flags`), no en `len_n_flags` de iwlwifi.
[`iwl_get_nvm`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-nvm-parse.c) acepta rsp v4 = 468 B.

**Fix aplicado:** quitar test sobre `len_n_flags`; test hostcheck `NVM_GET_INFO v4 (468 B, len=472)`.
**Placa pendiente:** `NVM_GET_INFO v4` ok → puerta TX_ANT/SF (run13).

### GPU-5. CE saxpy 512 B sin encoding boa0b5 — **confirmado (run14)**

**Síntoma:** tras G4e GO y G6 listo, `gsp_compute_stage_sass` (512 B) → semáforo no llega a 3
(vale 2) → `ce.stuck` → [`lx_nouveau_buf_ready`](lxdde/ports/nouveau/gsp_bringup.c) fuerza `pool VRAM=no`.

**soso:** [`gsp_ce.c`](lxdde/ports/nouveau/gsp_ce.c) usaba `LINE_LENGTH_IN = size` (512) sin `MULTI_LINE`
si size ≠ 4 KiB. [`gsp_compute.c`](lxdde/ports/nouveau/gsp_compute.c) copiaba 512 B crudos.

**Linux:** [`nouveau_boa0b5.c:58-70`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nouveau_boa0b5.c)
— `LINE_LENGTH = PAGE_SIZE`, `LINE_COUNT = PFN_UP(size)`, `MULTI_LINE_ENABLE`.

**Fix aplicado:** mismo encoding boa0b5; `stage_sass` pad + CE 4 KiB. Hostcheck saxpy 512 B verde.
**Placa pendiente:** `SASS en VRAM` + `pool VRAM=sí`.

### GPU-6. grctx ATTRIBUTE_CB: tamaño de mapeo sin alinear — **confirmado (run14)**

**Síntoma:** `mapeo sin alinear va=0x8041000000 phys=0x5000000 size=0x851200` →
`contexto de GR sin promocionar`. `0x851200 & 0xfff = 0x200`.

**soso:** [`gsp_grctx.c`](lxdde/ports/nouveau/gsp_grctx.c) mapeaba `b->size` crudo de RM;
[`gsp_vmm_map_flags`](lxdde/ports/nouveau/gsp_vmm.c) exige alineación 4 KiB.

**Linux:** [`r535/gr.c`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/gr.c)
mapea `nvkm_memory_size(pmem)` (objeto ya alineado).

**Fix aplicado:** `grctx_map_bytes()` redondea a `2^page_shift` (0x851200 → 0xA00000). Hostcheck verde.
**Placa pendiente:** `contexto de GR promocionado`.

### GPU-1. Parche BAR1 con PDB PRI-error mata el RPC — **confirmado (run13; corregido en run14 USB)**

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

### run13 (BAR1 / pool / GR0 / TX_ANT) — en USB antes de run14

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | No escribir PTEs BAR1 si PDB PRI-error | `gsp_bar1.c`, `gsp_bringup.c` | Placa run14: BAR1 no escribe; GR0 chid=2 ok |
| 2 | `gsp_buf_init` tras G4e, antes de GR0 | `gsp_bringup.c` | Placa run14: G6 listo (pool aún no por CE stuck) |
| 3 | Cachear method buffer; chid=2 | `gsp_chan.c` | Placa run14: GR0 + compute ok |
| 4 | SF async + TX_ANT sync | `iwl_mvm_up.c`, `iwl_trans.c` | Pendiente placa (run14 no llegó a TX_ANT) |

### run14 (NVM / CE / grctx) — **implementado en árbol; placa pendiente**

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | No tratar bit 6 de `len_n_flags` como rechazo FW | `iwl_trans.c`, `hcmd_contract_test.c` | Hostcheck OK. Placa: NVM_GET_INFO v4 |
| 2 | CE copias siempre a página (boa0b5) | `gsp_ce.c`, `gsp_compute.c`, `gsp-hostcheck` | Hostcheck OK. Placa: SASS + pool VRAM=sí |
| 3 | Mapeo grctx alineado a página | `gsp_grctx.c` | Hostcheck OK (0x851200→0xA00000). Placa: GR promocionado |

Reflash kernel (usuario):

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

## Qué no se ha hecho

- No se ha reflasheado el USB con los fixes run14.
- No `record-boot` / `--boot-ok` (`arranques_consecutivos_ok` = 0 en hw-matrix).
- Hostchecks verdes no sustituyen validación en placa (NVM, CE SASS, grctx, TX_ANT).
- No se ha validado `ask` con pool VRAM ni asociación WPA2.
