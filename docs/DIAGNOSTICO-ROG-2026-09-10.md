# ROG: GPU GA107 y WiFi AX200 — diagnóstico (10 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl` + `DBUS_SESSION_BUS_ADDRESS`. Copias:

- run17: `target/usb-diagnostic-2026-09-10-run5/` (SOSOLOG/SOSODRV = hueco virgen `0x0A`)
- run16: `target/usb-diagnostic-2026-09-10-run4/` (SOSOWIFI vacío; sin PSK)
- run15: `target/usb-diagnostic-2026-09-10-run3/`
- run14: `target/usb-diagnostic-2026-09-10-run2/`
- run13: `target/usb-diagnostic-2026-09-10/`

ESP desmontada al terminar.

| Campo | run17 (último) | run16 | run15 |
|---|---|---|---|
| Kernel USB | ELF `e6f491ec9-dirty` / 0.2.2 (strings) | **0.2.2 (e6f491ec9-dirty)** | 0.2.2 (276404696-dirty) |
| Flush / uptime | **ninguno** (SOSOLOG 256 KiB de `\n`) | **#28, 454 s** | #45, 1071 s |
| Hardware | sin SOSODRV (hueco virgen) | 10de:249c + 8086:2723 + 10ec:8168 | igual |
| sosh | **NO** | **SÍ** (+ `wifi scan` + `ask hola`) | SÍ |
| GSP / WiFi | no se alcanzó fatlog | GSP+pool ok; DQA timeout | GSP ok; TX_ANT timeout |
| BOOTMARK | **sí** (shim AMI 2.70; `bootsoso.efi` 156672 B) | (no copiado) | — |
| fatlog | **no escribió** | sí | sí |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/fw.c`, `mvm/sf.c`, `pcie/tx-gen2.c`, `iwl-csr.h`, `iwl-nvm-parse.c` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** — `r535/fifo.c`, `r535/bar.c`, `r570/fifo.c` |

Hostchecks (host, sin silicio): `l6-iwl-fw-hostcheck.sh` OK (DEF_ID cmd_ver TX_ANT=1 PHY=4,
omit DQA sin CAPA_DQA, up_minimal sin grp=5),
`l6-g3-gsp-hostcheck.sh` OK (SASS staging antes GR0 + grctx map + CE page-aligned).
Tras fixes run16 (omit DQA + cmd_ver DEF_ID): hostchecks verdes; **placa run17 no llegó a sosh** (no valida DQA).

## Tabla de etapas (run17)

| Etapa | Evidencia | Resultado |
|---|---|---|
| Shim UEFI | `BOOTMARK.TXT`: `soso-shim: UEFI alcanzado`; AMI rev 0x50013; `bootsoso.efi leído (156672 bytes); saltando al bootloader` | **OK** |
| Kernel / fatlog | `SOSOLOG.TXT` 262144 B, único byte `0x0A`; `strings` vacío | **FAIL** |
| hwscan | `SOSODRV.TXT` igual (hueco virgen) | **FAIL** |
| Userspace | no `sosh —` | **FAIL** |
| GSP / WiFi | sin líneas | no se ejecutó (o no se volcó) |

`fatlog::init()` está **después** de `boot: live-disk` ([`kernel/src/main.rs`](kernel/src/main.rs)). Un SOSOLOG virgen significa: el kernel no llegó a `fatlog::init()`, o `espfat::locate` falló (fichero no contiguo / tamaño distinto), o el bootloader no entregó el kernel.

**No es un defecto iwl/DQA confirmado:** ese código corre en `lxdde` / `boot: ethernet`, después de varios `flush_checkpoint`. Si el hang fuera ahí, el SOSOLOG tendría al menos `boot: memtest`…`live-disk`.

## Hallazgo run17 — `--only kernel` pisa la ESP live con la FAT de 31 MiB

**Síntoma:** tras `flash-usb-live --only kernel`, el shim corre y el kernel no deja rastro en SOSOLOG. run16 con el mismo `e6f491ec9-dirty` sí llegó a sosh.

**soso:** [`flash_usb_live.rs:198-212`](xtask/src/flash_usb_live.rs) hace `dd_partition(soso-uefi.img, p1 → USB p1)` y luego `create_esp_slots`. `target/soso-uefi.img` es un GPT de **31.1 MiB** (`sgdisk`: p1 34–63521 = **31.0 MiB**, 63488 sectores). El USB live tiene p1 **96 MiB** (`lsblk`). `dd_partition` **permite** destino más grande ([`package_live.rs:713-717`](xtask/src/package_live.rs) solo rechaza destino *más pequeño*). Copia 31 MiB de FAT (BPB de volumen ~31 MiB) sobre el inicio de una ESP de 96 MiB que ya tenía huecos 8.3 (`kernel-x86_64` 32 MiB, `SOSOKRN.BIN` 64 MiB).

El empaquetado live correcto ([`package_live.rs:91-97`](xtask/src/package_live.rs)) pega `soso-uefi.img` *al inicio de la imagen GPT* y luego `sgdisk` añade p2/p3/p4; la ESP live de este stick se había **estirado a 96 MiB**. `--only kernel` no actualiza `kernel-x86_64` in situ: sustituye el filesystem.

Ya existe la API correcta: [`fat32_write::overwrite_in_dir`](xtask/src/fat32_write.rs) (la usa `install_boot_shim` para `BOOTX64.EFI`). El test [`incremental_p1_preserves_p3`](xtask/src/flash_usb_live.rs) cubre src=dst del mismo tamaño (32 MiB), no el caso 31→96.

**Linux:** no hay árbol fat en `lxdde/linux/`. Un BPB de 31 MiB sobre partición de 96 MiB es un volumen inconsistente: el montaje host (vfat) puede listar entradas del directorio nuevo mezcladas con clusters viejos; UEFI `LoadImage` usa el tamaño FAT del fichero. Compatible con: shim OK (está en los primeros 31 MiB) + kernel que no arranca o fatlog que no localiza el hueco contiguo de 256 KiB.

**Confirmado** (tamaños + código + SOSOLOG virgen + BOOTMARK). Los cambios DQA/TLV 30 **no** explican un SOSOLOG vacío.

## Tabla de etapas (run16)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `wifi scan` + `ask hola` | **OK** |
| fatlog | flush **#28** @ 454 s | OK |
| lxdde pci / GSP | `GSP_INIT_DONE`, vaspace gp100 | **OK** |
| Canal COPY0 / CE G4e | `CE readback verificado (G4e GO)` | **OK** |
| G6 / pool | `SASS en VRAM`; `pool VRAM=sí` | **OK** |
| Canal GR0 | `RM_ALLOC chid=2`; compute `0xc7c0` | **OK** |
| grctx | `contexto de GR promocionado: 9 entradas` | **OK** |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE_NOTIF` | **OK** |
| WiFi NVM | `NVM_GET_INFO v4 nvm_ver=0x1235` | **OK** |
| WiFi TX_ANT | `TX_ANT_CONFIGURATION ok (ant=0x3)` | **OK** (vs run15) |
| WiFi DQA | `timeout cmd grp=5 id=0x00` → `DQA no habilitado` | **FAIL** |
| WiFi scan | `wifi scan: error de E/S` | **FAIL** |

**Interpretación run16:** fixes run15 (DEF_ID wire + SASS pre-GR0) **validados en placa** (TX_ANT ok, pool VRAM=sí).
Bloqueo WiFi: `DQA_ENABLE` mandado sin `CAPA_DQA_SUPPORT` — ucode `cc-a0-77` no declara bit 12 ni CMD_VERSIONS grp=5/cmd=0.
Linux [`mvm/fw.c:1601`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c) omite DQA si falta capa.
Segundo fix: `iwl_fw_cmd_ver` buscaba grupo 0 en TLV (PHY v4 en grp=1) — Linux [`fw/img.c:13`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/img.c).

## Tabla de etapas (run15)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `wifi scan` + halt | **OK** |
| fatlog | flush **#45** @ 1071 s | OK |
| lxdde pci / GSP | `GSP_INIT_DONE`, vaspace gp100 | **OK** |
| Canal COPY0 / CE G4e | `CE readback verificado (G4e GO)` | **OK** |
| G6 / pool | `G6 — buffers VRAM listos`; luego `pool VRAM=no` (`ce.stuck`) | **FAIL** |
| SASS saxpy | Tras GR0 en runlist 0xc00000: `SASS de saxpy no llegó a VRAM` | **FAIL** |
| Canal GR0 | `RM_ALLOC chid=2`; compute `0xc7c0` | **OK** |
| grctx | `contexto de GR promocionado: 9 entradas` | **OK** (vs run14) |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE_NOTIF` | **OK** |
| WiFi NVM | `NVM_GET_INFO v4 nvm_ver=0x1235`; MAC NVM real | **OK** (vs run14) |
| WiFi TX_ANT | `timeout cmd grp=0 id=0x98` tras SF async | **FAIL** |
| WiFi scan | `wifi scan: error de E/S` | **FAIL** |
| Apagado | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` | **OK** |

**Interpretación run15:** run14 fixes (NVM len_flags, CE page, grctx map) **ya en el USB**.
Bloqueos restantes: (1) HCMD legacy con `group_id=0` — Linux 6.6 usa `DEF_ID` → `LONG_GROUP=1` para TX_ANT/SF;
(2) primera copia CE de SASS **después** de programar GR0 en la runlist 0xc00000 deja el CE atascado
(aunque G4e fue GO y grctx promociona).

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

### run14 (NVM / CE / grctx) — **en USB run15**

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | No tratar bit 6 de `len_n_flags` como rechazo FW | `iwl_trans.c`, `hcmd_contract_test.c` | Placa run15: NVM_GET_INFO v4 ok |
| 2 | CE copias siempre a página (boa0b5) | `gsp_ce.c`, `gsp_compute.c`, `gsp-hostcheck` | Placa run15: G4e GO; SASS post-GR0 sigue stuck |
| 3 | Mapeo grctx alineado a página | `gsp_grctx.c` | Placa run15: GR promocionado (9 entradas) |

### run15 (DEF_ID / SASS pre-GR0) — **validado en placa run16**

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | `LEGACY_GROUP` → `LONG_GROUP` (DEF_ID) en `send_hcmd` | `iwl_trans.c`, `hcmd_contract_test.c`, `hcmd_wide_test.c` | Placa run16: TX_ANT ok + `iwl_rx` 0x98 |
| 2 | CE copia SASS tras G4e/G6, **antes** de GR0 | `gsp_bringup.c`, `gsp_compute.c` | Placa run16: `SASS en VRAM` + `pool VRAM=sí` |

### run16 (omit DQA / cmd_ver DEF_ID) — **en árbol; placa no validada (run17 no arrancó)**

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | Parsear TLV 30; omitir `DQA_ENABLE` sin `CAPA_DQA_SUPPORT` (bit 12) | `iwl_fw.c`, `iwl_mvm_up.c`, `capa_dqa_test.c` | Hostcheck OK. Placa: bloqueada por run17 |
| 2 | `iwl_fw_cmd_ver`: LEGACY_GROUP → LONG_GROUP en lookup | `iwl_fw.c`, `main.c` hostcheck | Hostcheck OK. Placa: bloqueada por run17 |

### run17 (`--only kernel` no debe dd la FAT de 31 MiB) — **implementado en árbol; placa pendiente**

| # | Cambio | Archivos | Hecho host / placa |
|---|---|---|---|
| 1 | `--only kernel`: sobrescribir in situ `KERNEL~1` + `efi/boot/{bootsoso,bootx64}.efi` con `overwrite_in_dir`; **no** `dd_partition` de `soso-uefi.img` p1 | `flash_usb_live.rs`, `package_live.rs` (`update_esp_from_uefi`), `fat32_write.rs` | Host: `only_kernel_in_place_preserves_96m_esp` verde. Placa: SOSOLOG con `boot:` + `sosh —` |

Este stick quedó con la FAT de 31 MiB pisada. `--only kernel` ahora actualiza el ELF que haya (`KERNEL~1`); si el volumen sigue siendo el de 31 MiB, fatlog puede fallar. Si tras el reflasheo no hay `boot:` en SOSOLOG, hace falta un flash completo (sin `--only`).

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

## Qué no se ha hecho

- `--only kernel` in situ está en el árbol; **placa pendiente** (reflash).
- omit DQA / cmd_ver DEF_ID **no** se ha visto en placa (run17 no llegó a sosh).
- No `record-boot` / `--boot-ok` (`arranques_consecutivos_ok` = 0).
- No scan BSS ni WPA2.
- `ask hola` en run16 seguía en CPU (sin `saxpy en GPU OK`).

## Ciclo del 10/09 (tarde): todo lo comprobable sin placa

No se ha flasheado nada en este ciclo. Lo que sigue es lo que cambia **antes**
del próximo arranque, con lo que hay que mirar en él.

### Lo que este ciclo deja medible en el siguiente arranque

| Qué mirar en el log | Por qué |
|---|---|
| `sonda CE — antes de GR0=…, tras crear GR0=…, tras PROMOTE_CTX=…` | Dice si el CE muere al crear el canal de GR o en el promote. Run15 solo permitía ver «murió en algún momento después». |
| `compute — familia X, SASS sm_YY, QMD …` | En GA107 debe salir `familia Ampere, SASS sm_86` y QMD «SIN layout en el árbol». Antes se lanzaba un descriptor de Blackwell con código sm_120. |
| `iwl_trans: timeout … slot=N; MVM parado` + `recuperación #1` | Un timeout ya no deja la cola en un estado del que no se sale: bloquea y el siguiente scan reinicia el transporte. |
| `SCAN_REQ_UMAC … origen=N pasivo=M` | `origen` distingue NVM, MCC, fallback y perfil vacío; `pasivo` dice si el scan es solo de escucha. |
| `MCC aplicado XX status=… canales=N válidos=M` | Antes MCC solo imprimía status y marcaba `mcc_done` sin aplicar la lista. |
| `wifi scan` con causa | Vacío normal, aborto, timeout, sin canales y sin regdominio se distinguen. |
| `SOSOHASH.TXT` en la ESP | Identidad de lo flasheado (kernel/rootfs/perfil por sha256). Copiarlo junto a SOSOLOG e importarlo con `--sosohash`. |

### Cambios de este ciclo (host/QEMU verde, placa pendiente)

| # | Cambio | Archivos | Comprobado |
|---|---|---|---|
| 1 | RX descarta el paquete cuyo `len` anunciado supera lo recibido; `cmd_resp_trunc` | `iwl_trans.c`, `iwl_mvm_nvm.c`, `hcmd_contract_test.c` | Host + ASan/UBSan |
| 2 | Cola HCMD FIFO con propiedad por slot, backpressure y recuperación | `iwl_trans.c`, `iwl_ax211.c`, `wifi.rs`, `hcmd_queue_test.c` | Host: 31 async, orden, reentrada, recover |
| 3 | MCC completo + política única de canales + banda v17 en `flags[31:30]` | `iwl_mvm.c`, `iwl_mvm_nvm.c`, `mcc_chan_test.c` | Host |
| 4 | BSS clasificado por beacon (Privacy/RSN/AKM/CCMP) y SSID exacto | `iwl_mvm.c`, `bss_select_test.c` | Host |
| 5 | Juegos de SASS por arquitectura + rechazo pre-submit por familia | `gsp_compute.c/h`, `gsp_sass.h`, `sass_sm86.c`, `sass_sm120.c` | Host (banco GSP) |
| 6 | Tres medidas de CE con volcado de estado en el primer fallo | `gsp_bringup.c` | Comprobado sobre el fuente en el banco |
| 7 | Identidad de arranque por banner; racha derivada del historial | `xtask/src/hw_matrix.rs` | 27 tests; matriz remigrada 14 → 28 |
| 8 | `SOSOHASH.TXT` + `parse-logs --sosohash` | `package_live.rs`, `hw_matrix.rs` | Test de xtask |
| 9 | Contrato de mremap con reserva y rollback | `addrspace.rs`, `syscall.rs`, `init` | QEMU: `init test` |

### Qué no se ha hecho (sigue igual)

- No se ha reflasheado el USB: nada de esto se ha visto en placa.
- Scan con BSS, asociación 802.11 y WPA2 siguen sin AP controlado.
- Compute en GA107 seguirá rechazado hasta tener el descriptor QMD de Ampere.
