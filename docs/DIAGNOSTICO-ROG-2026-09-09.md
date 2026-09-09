# ROG: GPU GA107 y WiFi AX200 — diagnóstico (9 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`kernel`, vfat) con `udisksctl`. Última copia:
`target/usb-diagnostic-2026-09-09-run12/` (sin PSK en SOSOWIFI). ESP desmontada al terminar.

| Campo | run12 (último USB) | run11 | run10 | run9 |
|---|---|---|---|---|
| Kernel USB | **0.2.2 (1df51e817-dirty)** | 0.2.2 (1df51e817-dirty) | 0.2.2 | 0.2.2 |
| Flush / uptime | **#43, 1024 s** | #39, 432 s | #5, 141 s | #4, 140 s |
| Hardware | 10de:249c + 8086:2723 + 10ec:8168 | igual | igual | igual |
| sosh | **SÍ** (+ wifi scan + halt) | SÍ | NO | NO |
| GSP | **GSP_INIT_DONE + PRAMIN 0x001700** | igual | no alcanzado | no |
| GPU bloqueo | **RM_ALLOC 0xc56f NO_MEMORY chid=0** | NO_MEMORY | — | — |
| WiFi | **ALIVE + SF ok**; timeout **TX_ANT/PHY/MCC/SCAN_CFG** | SCAN_CFG timeout | register only | — |
| fatlog checkpoints | sí | sí | sí | sí |

Copias anteriores: run11/run10/run9 en `target/usb-diagnostic-2026-09-09-run11/` etc.

## Tabla de etapas (run12)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `wifi scan` + `halt` limpio | **OK** |
| fatlog | flush **#43** @ 1024 s | OK |
| lxdde pci / GSP | `GSP_INIT_DONE`, vaspace gp100 VRAM, PRAMIN `0x001700` | **OK** |
| Canal CE | `RM_ALLOC cls=0xc56f` → NO_MEMORY (0x51) `chid=0 flags=0x00200020` | **FAIL** |
| VRAM pool | `pool VRAM=no` | **FAIL** |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, `SHARED_MEM_CFG ok`, `SF_INIT_OFF` | **OK** |
| WiFi cmds | timeout `TX_ANT`/`PHY_CONTEXT`/`MCC`/`SCAN_CFG` (legacy 4 B tras SF) | **FAIL** |
| Scan userspace | `sosh: wifi scan: error de E/S` | **FAIL** |

**Interpretación run12:** NVOS04 flags y `iwl_mvm_up_minimal` del USB **no bastaron**.
Bloqueos raíz: (1) `chid=0` en GPFIFO — Linux `rsvd_chids=1` (`r570/fifo.c:212`);
(2) HCMD grupo 0 con cabecera legacy 4 B — Linux gen2 siempre wide (`tx-gen2.c`);
(3) secuencia MVM incompleta (falta DQA/rxq, `mvm_up_done` pese a timeouts).
Correcciones aplicadas en árbol en esta sesión (hostchecks verdes).

## Tabla de etapas (run11)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —` + `boot: task` | **OK** |
| fatlog | flush **#39** @ 432 s | OK |
| lxdde pci / GSP | `BAR0 boot0=0xb74000a1`, `GSP_INIT_DONE`, vaspace gp100 VRAM | **OK** |
| Canal CE | `RM_ALLOC cls=0xc56f` → NO_MEMORY (0x51) | **FAIL** |
| VRAM pool | `pool VRAM=no` | **FAIL** |
| WiFi alive | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE_NOTIF`, `alive=true` | **OK** |
| WiFi cmds | timeout `TX_ANT`/`MCC`/`SCAN_CFG` | **FAIL** |
| Scan userspace | `sosh: wifi scan: error de E/S` | **FAIL** |

**Interpretación:** run11 valida probe lock + PRAMIN Ampere (run10→run11). Nuevo bloqueo:
alloc canal GPFIFO (`r535_chan_alloc` flags) y fase MVM up antes de TX_ANT/scan
(Linux `iwl_mvm_up` L1544–1696). Correcciones en árbol en esta sesión.

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | 6.6.32 — `mvm/fw.c`, `mvm/scan.c`, `fw/api/scan.h` |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf` — `r535/vmm.c`, `vmmgp100.c`, `r535/gsp.c` |

Hostchecks (run10, tras fix PRAMIN Ampere): `l6-g3-gsp-hostcheck.sh` OK
(`check_pramin` Ampere 0x001700 + Blackwell 0x10fd40, `check_vmm_ampere` VIDMEM+PD VRAM),
`l6-iwl-fw-hostcheck.sh` OK (sin PHY_CFG 0x0b).
No cubren transporte Ampere ni scan en placa.

## Tabla de etapas (run10)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | sin `sosh` | **FAIL** (cuelgue) |
| fatlog | flush **#5** @ 141 s | OK |
| lxdde module | `lxdde: nouveau init rc=0 phase=none` + self-test nvkm OK | OK (registro driver) |
| lxdde pci / GSP | sin `nouveau-lx: probe`, sin `BAR0 boot0`, sin `GSP_INIT_DONE` | **NO ALCANZADO** |
| WiFi | `lxdde: iwlwifi register rc=0`; sin `start`, sin `UCODE_ALIVE_NTFY` | **Parcial** (register OK) |
| Ethernet | `boot: ethernet` alcanzado; no `boot: red` | Parcial |

**Interpretación:** run10 avanza respecto a run9: el flush #5 confirma que
`iwlwifi register` terminó. El cuelgue está en **`pci::init` → `nouveau_probe` →
`lx_nouveau_gsp_init`** con `DRIVERS.lock()` sostenido durante todo el probe
(más de 60 MiB de firmware por USB). Corrección en árbol: soltar el lock antes
del probe, ventana PRAMIN Ampere en `0x001700` (no `0x10fd40`), flush tras BAR0.

## Tabla de etapas (run9)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | sin `sosh` | **FAIL** (cuelgue) |
| fatlog | flush **#4** @ 140 s; checkpoints `boot: kbd`, `boot: fs` visibles | OK (ya no ciego pre-lxdde) |
| lxdde module | `lxdde: nouveau init rc=0 phase=none` + self-test nvkm OK | OK (registro driver) |
| lxdde pci / GSP | sin `nouveau-lx: probe`, sin `BAR0 boot0`, sin `GSP_INIT_DONE` | **NO ALCANZADO** (en log USB) |
| WiFi | sin `lxdde: iwlwifi register/start`, sin `UCODE_ALIVE_NTFY` | **NO ALCANZADO** |
| Ethernet | `boot: ethernet` alcanzado; no `boot: red` | Parcial |

**Interpretación:** el flush #4 coincide con el checkpoint tras `lxdde: nouveau init`
(`kernel/src/lxdde/mod.rs`). Todo lo posterior (`wifi::init`, `pci::init` →
`nouveau_probe` → `lx_nouveau_gsp_init`) quedó **solo en logbuf RAM** hasta el
cuelgue — el SOSOLOG en ESP no lo capturó. Run8 se atascaba antes (flush #1);
run9 avanza gracias a los checkpoints pero el bloqueo sigue en la fase
**pci::init / bring-up GSP Ampere** (hipótesis fuerte, coherente con run7/run8).

## Tabla de etapas (run8)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | sin `sosh` | **FAIL** (cuelgue) |
| boot: iommu | `boot: iommu` antes de `pci` | OK |
| fatlog | flush #1 @ 109 s; última línea `fatlog: SOSOLOG.TXT LBA 194461` | **CIEGO** |
| drvlog | SOSODRV distinto de run7 (HID 048d / 0b05) | OK (post-flush #1) |
| GPU / WiFi | sin líneas GSP ni iwlwifi | **NO ALCANZADO** |
| Ethernet | no llegó a `boot: red` | — |

Run8 lleva el fix `numEntries=4` pero aún tenía PD en sysmem + aperture SYSMEM_COH.
El cuelgue encaja con RM caminando el directorio como VRAM. SOSODRV confirma que
`drvlog::init` corrió **después** del único flush de fatlog.

## Tabla de etapas (run7)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh — escribe 'help'` | OK |
| GPU: FWSEC-FRTS + booter | `booter_load boot ok`, WPR2, heap **128 MiB** | OK |
| GPU: GSP-RM / RPC | `GSP_INIT_DONE` tras 6 msgs; `cpu_seq 420 comando(s)` | **OK** |
| GPU: pool VRAM | `SET_PAGE_DIRECTORY` → `INVALID_ARGUMENT (0x1f)`; `pool VRAM=no` | **FAIL** |
| WiFi: ALIVE | `UCODE_ALIVE_NTFY` | OK |
| WiFi: INIT_COMPLETE | `INIT_COMPLETE_NOTIF` | OK |
| WiFi: init MVM | `timeout cmd grp=0 id=0x0b`; `PHY_CONFIGURATION falló` | **FAIL** |
| WiFi: scan | `scan sin INIT_COMPLETE` (sin `radio_ready`) | **FAIL** |
| Ethernet | rtl8169 DOWN | Sin DHCP |

## Progreso respecto a run8

Run8: flush #1, cuelgue tras `live-disk` sin checkpoints intermedios. Run9 (mismo
commit dirty, kernel con `flush_checkpoint`): flush #4, llega a `boot: fs`, monta
sosomfs, registra nouveau (`phase=none`) y se congela. Los checkpoints confirman que
el SOSOLOG ya no queda ciego **antes** de lxdde; falta volcar **después** de
`pci::init`/GSP.

## Progreso respecto a run7

Run6 llevaba fixes cpu_seq 64 KiB, CORE_RESET falcon, PHY_CFG mandado en AX200, SCAN_CFG v5.
En run7 (`1df51e817`) los dos primeros **funcionan** (GSP_INIT_DONE). Los bloqueos nuevos son:

1. **VMM Ampere** — `SET_PAGE_DIRECTORY` con geometría VER3 (`numEntries=2`) rechazada en GA107.
2. **PHY_CFG** — el fix run6 de mandar `PHY_CONFIGURATION_CMD` 0x0b **rompe** el init unificado
   AX200: el FW ya envió `INIT_COMPLETE` y el 0x0b queda sordo.

## Hallazgos confirmados (run7)

### Hallazgo run9 — cuelgue post-nouveau (pci/GSP no volcado)

**Síntoma:** SOSOLOG USB termina en `lxdde: nouveau init rc=0 phase=none` (flush #4).
No hay `nouveau-lx: probe`, `BAR0 boot0`, `GSP_INIT_DONE`, ni `lxdde: iwlwifi`.

**soso:** Tras el checkpoint en [`kernel/src/lxdde/mod.rs`](kernel/src/lxdde/mod.rs) (~L107),
siguen `wifi::init()` → `pci::init()` → `nouveau_probe` → `lx_nouveau_gsp_init`
([`gsp_bringup.c:915`](lxdde/ports/nouveau/gsp_bringup.c)). El cuelgue encaja con
**`pci::init` / bring-up Ampere**; las líneas posteriores no llegaron al ESP porque
no hay flush tras `iwlwifi register` ni tras `pci::init`.

**soso (riesgo):** [`kernel/src/lxdde/pci.rs:54-97`](kernel/src/lxdde/pci.rs) mantiene
`DRIVERS.lock()` durante todo el bucle de `probe` — si el bring-up GSP reentrara en
`lx_pci_register_driver`, habría deadlock. Hoy sólo nouveau registra driver en probe
path, pero conviene soltar el lock antes de `probe`.

**Linux:** Ampere GSP arranca por SEC2/booter + cadena RPC (`ga102.c`, `r535/gsp.c` en
`lxdde/reference/linux-master-nouveau/` @ fc02acf — árbol no presente en checkout;
referencia documentada en run7). El host no debe bloquearse indefinidamente en MMIO:
timeouts acotados (`tu102_devinit_wait`, `gsp_rpc_wait_event` 4 s en soso L890).

**Confirmado vs hipótesis:** Confirmado el atasco **después** del registro nouveau y
**antes** de evidencia WiFi/GSP en log USB. Causal exacto (deadlock vs poll MMIO vs
VMM/PRAMIN) **pendiente** hasta añadir checkpoints post-pci.

### GPU-3. VMM Ampere (en árbol, no validado run9)

**soso (run7):** [`gsp_vmm.c`](lxdde/ports/nouveau/gsp_vmm.c) fijaba VER3 con `numEntries=2`.

**Linux Ampere:** `tu102_vmm` / `gp100_vmm_desc_12` → `numEntries = 1 << bits = 4`
([`r535/vmm.c:142`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/rm/r535/vmm.c),
[`vmmgp100.c`](lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/mmu/vmmgp100.c)).

**Corrección aplicada (run8→run9):** raíz gp100 en VRAM (PRAMIN), `FLAGS.APERTURE =
VIDMEM`, invalidate `tu102_vmm_flush` (`phys>>8`, sin OR 0x2), PTE VRAM solo VALID.
Hostcheck: `check_vmm_ampere` + PTE codificación manual.

### WiFi-3. Omitir PHY_CFG en ucode unificado (AX200/AX211)

**soso (run7):** [`iwl_mvm_init.c`](lxdde/ports/iwlwifi/iwl_mvm_init.c) mandaba
`PHY_CONFIGURATION_CMD` 0x0b tras `INIT_COMPLETE_NOTIF`.

**Linux:** `iwl_send_phy_cfg_cmd` return 0 sin enviar si ucode unificado y no SISO
([`fw.c:539-541`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)).

**Corrección aplicada:** omitir PHY_CFG; tras `INIT_COMPLETE` marcar `radio_ready=1` y
seguir NVM MAC / TX_ANT / MCC. Hostcheck: `mvm_init_hostcheck` verifica ausencia de 0x0b.

### Hallazgos run6 (ya en árbol, validados en run7)

| # | Cambio | run7 |
|---|---|---|
| GPU-1 | Buffer cpu_seq 64 KiB | OK (`420 cmds`) |
| GPU-2 | CORE_RESET = falcon reset | OK (GSP_INIT_DONE) |
| WiFi-2 | SCAN_CFG v5 = 12 B | No alcanzado (init MVM incompleto) |

## Orden de corrección (estado)

| # | Cambio | Estado |
|---|---|---|
| 1 | VMM gp100 Ampere (VIDMEM + PD VRAM + invalidate tu102 + PTE) | **En árbol**; **placa run11 OK** (vaspace gp100 VRAM) |
| 2 | Omitir PHY_CFG ucode unificado AX200 | **En árbol**; **placa run11 OK** (sin 0x0b) |
| 3 | Flush fatlog + soltar probe lock + PRAMIN `0x001700` | **En árbol**; **placa run11 OK** (GSP_INIT_DONE, sosh) |
| 4 | `chid = idx + rsvd_chids(1)` + log inst/userd/mthdbuf (`gsp_chan.c`) | **En árbol** (hostcheck OK); placa pendiente post-reflash |
| 5 | HCMD gen2 siempre wide + `iwl_mvm_up` (DQA, rxq, 2000 ms, SCAN_CFG) | **En árbol** (hostcheck OK); placa pendiente post-reflash |
| 6 | Validar placa: `RM_ALLOC 0xc56f ok`, `pool VRAM=sí`, `SCAN_CFG ok`, scan BSS | Pendiente (run13 tras reflash kernel) |

Reflashear (usuario):

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

## Qué no se ha hecho

- No se ha reflasheado el USB con las correcciones run12 (chid rsvd, HCMD wide, mvm_up DQA).
- Hostchecks verdes no sustituyen validación `RM_ALLOC` canal ni scan en placa.
- No se ha ejecutado `record-boot` ni `--boot-ok` (run12 tiene sosh pero GPU/scan fallan).
