# ROG: GPU GA107 y WiFi AX200 — diagnóstico (11 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`kernel`, vfat 192 MiB) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-11/` (SOSOLOG/SOSODRV recortados). ESP desmontada al terminar.

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`29056871b-dirty`)** — banner en SOSOLOG |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200) + `10ec:8168` (rtl8169) |
| Userspace | **`sosh —`** alcanzado |
| fatlog | flush **#27** @ ~116 s (`uptime=116210ms`) |
| ESP | 192 MiB intacta |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/mac-ctxt.c`, `fw/api/mac.h`, `fw/api/commands.h` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** — `ga100.c`, `ga1xx.c`, `tu102.c`, `r535/gr.c` |
| OGKM | **570.144** — `clc7c0.h`, `cla0c0qmd.h` |

Hostchecks (host, tras fixes de este ciclo): `l6-iwl-fw-hostcheck.sh` OK
(MAC_CONTEXT 0x28 LEGACY→grp=1, 148 B, filter@52 qos@56),
`l6-g3-gsp-hostcheck.sh` OK (stride VA 0x20000, ventanas idx=0/1 disjuntas).

## Tabla de etapas (11-sep, kernel `29056871b-dirty`, flush #27)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` | **OK** |
| fatlog | flush #27 @ ~116 s | OK |
| lxdde pci / GSP | `GSP_INIT_DONE`, vaspace | **OK** |
| CE picker COPY2 | `CE bring-up motor 11` | **OK** |
| Canal GPFIFO | BIND motor 11, SCHEDULE ok | **OK** |
| CE DMA_COPY ALLOC | `RM_ALLOC cls=0xc7b5 ok (8 B)` | **OK** |
| CE doorbell COPY2 | `doorbell ajustado 0x00000001 → 0x00010001` | **OK** |
| CE selftest | `CE selftest OK`, `CE readback verificado (G4e GO)` | **OK** |
| G6 / pool (inicial) | `G6 — buffers VRAM listos` | **OK** |
| Golden GR | PROMOTE_CTX 9 entradas ok (cli `0xc1d00002`) | **OK** |
| CE tras GR0 | `sonda CE (tras crear GR0): NO señaliza`; sem 10/11; notifier pisado | **FAIL** |
| G6 / pool (final) | `pool VRAM=no` | **FAIL** |
| GR0 compute PROMOTE | `RM_CONTROL 0x2080012b → INVALID_ARGUMENT (0x1f)` | **FAIL** |
| WiFi alive | `UCODE_ALIVE_NTFY` | **OK** |
| WiFi init | `INIT_COMPLETE`, TX_ANT/PHY ok | **OK** |
| WiFi MAC_CONTEXT | `timeout cmd grp=1 id=0x28` (×4 recoveries) | **FAIL** |
| WiFi SCAN_CFG | cola bloqueada tras MAC timeout | **FAIL** |
| Ethernet | rtl8169 DOWN 10M half, sin DHCP | sin DHCP (no marcar ok) |
| Compute | `GPU sin pool de VRAM` + GR sin promocionar | **FAIL** |

**No marcar `--boot-ok`:** pool VRAM, compute, scan y MAC_CONTEXT siguen en fail.

## Fixes del ciclo anterior — validados en placa (#27)

1. **Doorbell Ampere COPY2:** kick `0x00010001` → CE selftest y readback inicial OK.
2. **MAC_CONTEXT grp=1:** ya no va por grp=3; el timeout persiste por layout del payload.

## Hallazgos confirmados (pendientes en #27)

### GPU-1. Solape VA CE/GR0 — stride 64 KiB

**Síntoma:** Tras golden, `sonda CE (tras golden GR): mueve`. Al crear GR0
(`gpfifo=0x8020010000`, idx=1) el CE (`pushbuf=0x8020008000`, `notifier=0x8020018000`)
deja de señalizar: `semáforo no llegó a 11 (vale 10)`; USERD `GPGet=11`; `pool VRAM=no`.

**Causa:** `GSP_CHAN_VA_STRIDE=0x10000` pero cada canal mapea 104 KiB
(GPFIFO 32 KiB + PB 64 KiB + notifier 4 KiB). idx=1 empieza a +64 KiB y solapa
PB/notifier de idx=0.

**Fix (este ciclo):** `GSP_CHAN_VA_STRIDE=0x20000` (≥ `GSP_CHAN_VA_USED=0x19000`).
Hostcheck: ventanas idx=0/1 disjuntas.

**Validación placa (pendiente reflash):** `sonda CE (tras crear GR0): mueve`;
`pool VRAM=sí`; CE no se rebinda.

### GPU-2. Golden oneinit libera cliente RM — 2º PROMOTE 0x1f

**Síntoma:** golden ok (9 entradas, cli `0xc1d00002`); luego `gsp_vmm_fini` del
VMM temporal; 2º promote (cli `0xc1d00001`) reusa físicas globales → `0x1f`.

**Causa:** soso creaba `golden_vmm` con client_id=2 y hacía FREE del cliente al
`gsp_vmm_fini`. Linux libera canal/VMM golden pero conserva `gr->ctxbuf_mem`.

**Fix (este ciclo):** golden sobre el mismo `g_vmm.rm` (canal idx=2); solo
`gsp_chan_fini(&golden)` al terminar — sin `gsp_vmm_fini` del cliente de globales.

**Validación placa:** `contexto de GR promocionado` en canal compute (sin `0x1f`).

### WiFi-1. MAC_CONTEXT — filter_flags en offset de qos_flags

**Síntoma:** PHY `id=0x08` grp=1 OK; `timeout cmd grp=1 id=0x28 slot=7` en up y
en 3 recoveries; MCC/SCAN_CFG en cascada (cola bloqueada).

**Causa:** soso escribía filtros en `cmd+56` (`qos_flags` en Linux `mac.h`).
Faltaban `ac[]` EDCA y `sta.is_assoc=0` para scan.

**Fix (este ciclo):** struct `iwl_mac_ctx_cmd` empaquetada (148 B); `filter_flags`
@+52; defaults EDCA (`cw_min=0x0f`, `cw_max=0x3f`, `aifsn=1`); hostcheck offsets.

**Validación placa:** `MAC_CONTEXT scan id=… ok`; `SCAN_CFG`; `wifi scan` con BSS.

## Correcciones implementadas (11-sep, sin reflash en placa)

1. **GPU:** stride VA canal `0x20000`; golden oneinit en mismo RM client.
2. **WiFi:** layout Linux de `MAC_CONTEXT` (filter@52, EDCA, union 148 B).
3. **Registro:** `hw-matrix.json` flush #27; este informe.

**Validación placa:** `sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel`
