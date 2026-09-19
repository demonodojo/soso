# ROG GA104 + AX200 — QMD Ampere y WiFi post-recover (18 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias:

- [`target/usb-diagnostic-2026-09-18/`](../target/usb-diagnostic-2026-09-18/) — flush **#104** (sesión larga + `ask`).
- [`target/usb-diagnostic-2026-09-18-run2/`](../target/usb-diagnostic-2026-09-18-run2/) — flush **#19** (arranque corto tras volver el USB).
- [`target/usb-diagnostic-2026-09-18-run3/`](../target/usb-diagnostic-2026-09-18-run3/) — flush **#17** (kernel **0.3.1** en placa; GSP + WiFi boot; sin `ask`).

Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | flush **#17** (run3) | flush **#19** (run2) | flush **#104** (run1) |
|---|---|---|---|
| Kernel en placa (SOSOLOG) | **0.3.1 (`07e6ef747-dirty`)** | **0.2.2 (`4f94c6cb9-dirty`)** | igual run2 |
| Flush / uptime | **#17 @ 119 141 ms** (~2 min) | **#19 @ 201 375 ms** (~3,4 min) | **#104 @ 19 797 566 ms** (~5,5 h) |
| Hardware | `10de:249c` GA104 + `8086:2723` AX200 + `10ec:8168` DOWN | igual | igual |
| Userspace | **`sosh —`** pid=2; *sin* `halt` | **`sosh —`** pid=2; **`halt`** | sosh + **`ask hola`** + halt |
| USB | **0b05:18c6** GET config **timeout** (slot 3); 0b05:1866 + stick BOT OK | (no registrado en run2) | — |
| WiFi (boot) | ALIVE; scan **23/24** BSS; 4-way; DHCP **192.168.68.132/24** LxWifi | ALIVE; scan **22** BSS; igual DHCP | igual en boot |
| WiFi (recon) | *no probado* | *no probado* | `dhcp perdido` → recover → **0 BSS**; EAPOL falló |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; CE readback; *sin* `matvec-res`/`ask` | igual | CE **435/435**; **matvec-res** QMD hang → CPU |
| Árbol en host (HEAD) | parches xHCI GET_CONFIG + QMD invalidate (esta sesión) | **0.2.3 (`7cf500d00`)** — QMD OGKM + 8814287dc | — |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — iwlwifi restart/rescan, nouveau GSP |
| OGKM 570.144 | `cla0c0qmd.h` (QMDV01_07), `clc7c0.h` (`SET_QMD_VERSION` 0x0288) |

Hostchecks (host, tras parches de este informe):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK**
- `./scripts/l6-g3-gsp-hostcheck.sh` — **OK**

No acreditan silicio tras reflash; el stick sigue con kernel **`4f94c6cb9-dirty`** hasta `--only kernel`.

Matriz: [`hw-matrix.json`](hw-matrix.json) — `ga107-igpu`, `ax200-wifi`; logs run3 + run2; **sin** `record-boot`.

---

## Tabla de etapas (flush #17, run3)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.3.1 (07e6ef747-dirty)` → `sosh —` pid=2 | **OK** |
| GPU GSP / pool / CE | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado` | **OK** |
| GPU compute / carga | sin `matvec-res` ni `ask` en el log | **pendiente** (fail heredado de #104) |
| WiFi | ALIVE; `SCAN_COMPLETE count=23/24`; 4-way; `net: dhcp 192.168.68.132/24` | **OK** |
| USB xHCI | `0b05:18c6`: device descriptor OK → `transfer event timeout` EP0 → `failed to get config descriptor` | **fail** (fix en árbol, pendiente reflash) |
| Reconexión | no ejercitada | **pendiente** (fail en #104) |
| Apagado | log termina en sosh | **pendiente** |

---

## Tabla de etapas (flush #19, run2)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (4f94c6cb9-dirty)` → `sosh —` | **OK** |
| GPU GSP / pool / CE | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado` | **OK** |
| GPU compute / carga | sin `matvec-res` ni `ask` en el log | **pendiente** (fail heredado de #104) |
| WiFi | ALIVE; `SCAN_COMPLETE count=22`; 4-way; `net: dhcp 192.168.68.132/24` | **OK** |
| Reconexión | no ejercitada | **pendiente** (fail en #104) |
| Apagado | `$ halt` | **OK** |

---

## Tabla de etapas (flush #104)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (4f94c6cb9-dirty)` → `sosh —` | **OK** |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí` | **OK** |
| GPU CE / G6 | subida **435/435** tensores | **OK** |
| GPU compute | `matvec-res` sem=0; `PBDMA_HANG_DURING_HTE`; `camino de GPU desactivado`; `on_gpu=0` | **fail** |
| GPU carga | `soso-llm: generado` 138 tok CPU; `44713 matvec` contados sin silicio | **fail** |
| WiFi ALIVE / init | `UCODE_ALIVE_NTFY`; `INIT_COMPLETE_NOTIF` | **OK** |
| Scan (boot) | `SCAN_COMPLETE count=17/23` | **OK** |
| 4-way / DHCP | `192.168.68.132/24` LxWifi | **OK** |
| Reconexión | `timeout cmd grp=1 id=0x0d` (SCAN); `'Rutilo' … 0 BSS` | **fail** |
| Apagado | `$ halt` | **OK** |

---

## Hallazgos y correcciones (orden aplicado)

### 1. QMD Ampere incompleto (bloqueante GPU)

**Síntoma:** primer `matvec-res` no señaliza; `GR_EXCEPTION` type=13; canal avanzó (`USERD GPGet=1`).

**Causa:** descriptor QMD **1.0** (`QMD_MAJOR_VERSION=1`, minor 0) y pushbuffer sin `NVC7C0_SET_QMD_VERSION` / `CHECK_QMD_VERSION` antes de `SEND_PCAS_A`.

**Cambio en soso (8814287dc):** `QMD_VERSION=7` en `gsp_compute_fill_qmd_v02_grid`; encoder emite **0x01070107**; hostcheck R5.3 + banco exigen minor=7 y PB de **40 B** (10 dwords).

**Avance vs [17 sep](DIAGNOSTICO-ROG-2026-09-17.md):** el hang CE G6 (`+64` / 1 PTE) **ya no aparece**; el bloqueo pasó al **primer QMD**.

### 1b. QMD Ampere vs OGKM — GROUP_ID y cbank (HEAD, pendiente placa)

**Síntoma:** mismo hang de #104 con QMD minor=0 en USB; tras 8814287dc en host aún faltaban campos respecto a Blackwell/OGKM.

**soso:** [`gsp_compute.c`](../lxdde/ports/nouveau/gsp_compute.c) `gsp_compute_fill_qmd_v02_grid` — sin `QMD_GROUP_ID`; cbank `(cb_va - GSP_VA_BASE) >> 6`.

**OGKM 570.144:** [`cla0c0qmd.h`](../lxdde/reference/open-gpu-kernel-modules-570.144/src/common/sdk/nvidia/inc/class/cla0c0qmd.h) — `NVA0C0_QMDV01_07_QMD_GROUP_ID` MW(197:192); `CONSTANT_BUFFER_ADDR_*` encajan **VA>>6** (40 bits), no offset relativo a la base del espacio.

**Cambio en soso (esta sesión):** `QMD_GROUP_ID=0x1f` (como v05); cbank `cb_va >> 6`; macro `QMDV02_QMD_GROUP_ID` en [`nvrm_r570.h`](../lxdde/ports/nouveau/nvrm_r570.h); R5.3 en [`tools/gsp-hostcheck/main.c`](../tools/gsp-hostcheck/main.c).

**Validación host:** `./scripts/l6-g3-gsp-hostcheck.sh` OK. **Placa:** tras flash, `ask hola` → `matvec-res` con semáforo y `on_gpu=1`.

### 1c. QMD invalidate caches (HEAD, pendiente placa)

**Motivo:** si tras GROUP_ID/cbank + QMD v7 el primer `matvec-res` sigue colgando, OGKM exige invalidar cachés de instrucción y constantes en QMD v01_07 (MW 254/255).

**Cambio en soso:** `INVALIDATE_INSTRUCTION_CACHE` y `INVALIDATE_SHADER_CONSTANT_CACHE` = TRUE en `gsp_compute_fill_qmd_v02_grid`; hostcheck R5.3 los exige.

**Validación host:** `./scripts/l6-g3-gsp-hostcheck.sh` OK. **Placa:** `ask hola` → `on_gpu=1` sin `camino de GPU desactivado`.

### 4. GET_DESCRIPTOR(Config) xHCI — ASUS 0b05:18c6 (run3)

**Síntoma (flush #17):** tras descriptor de dispositivo OK para **VID=0x0b05 PID=0x18c6** (slot 3, puerto 4), timeout EP0 5 s (`USBSTS=0x18`), `EP recovered`, `failed to get config descriptor for slot 3`. Teclado **0b05:1866** y mass-storage BOT enumeran.

**Referencia Linux 6.6.32:** `usb_get_descriptor` reintenta hasta 3 veces; `usb_get_configuration` lee cabecera 9 B y luego `wTotalLength`; `handle_port_status` ack CSC/PEC en PORTSC.

**Cambio en soso:** [`crates/xhci-nostd/src/driver.rs`](../crates/xhci-nostd/src/driver.rs) — reintentos header+full (3×, pausa ~200 ms) en `get_configuration_descriptor`; `PORT_STATUS_CHANGE` ack bits de cambio + `clear_pcd`.

**Validación host:** `cargo test -p xhci-nostd`. **Placa:** tras flash, el log **no** debe mostrar `failed to get config descriptor` para 0b05:18c6 (o el HID enumera).

### 2. Scan UMAC tras recover (bloqueante reconexión WiFi)

**Síntoma:** tras `timeout cmd … SCAN_REQ`, recover ALIVE pero **`scan_count=0`** al conectar.

**Cambio:** `iwl_trans_recover` limpia lista de BSS; `iwl_mvm_ensure_scan_for_connect` llama `iwl_mvm_scan` si `scan_count==0` antes de `pick_bss` (open y WPA2).

### 3. Parser matriz A8

**Síntoma:** `44713 matvec` marcaba compute/carga **ok** con `on_gpu=0` y `camino de GPU desactivado`.

**Cambio:** `gpu_compute_fallido` reconoce esas cadenas; `carga_real` exige `on_gpu=1` sin fallo GPU; fixture `xtask/fixtures/rog-2026-09-18-gpu-cpu.log`.

---

## Orden de corrección (pendiente en placa)

1. **Flash kernel HEAD** — `cargo xtask flash-usb-live /dev/sda --yes --only kernel` (8814287dc + QMD GROUP_ID/cbank + invalidate + xHCI GET_CONFIG retry/PORT ack).
2. **USB 0b05:18c6** — arranque con periférico conectado; criterio: sin `failed to get config descriptor for slot` (o HID usable).
3. **GPU matvec-res** — `ask hola`; criterio: sin `camino de GPU desactivado`, `on_gpu=1`, semáforo QMD. Si falla: `GR_EXCEPTION` / USERD vs hostcheck R5.3.
4. **WiFi reconexión** — provocar `dhcp perdido` + `wifi connect`; criterio: scan repoblado (no `0 BSS`) con `iwl_mvm_ensure_scan_for_connect`.
5. **Matriz** — tras evidencia nueva: `parse-logs` + revisión manual; no `record-boot` hasta 3 arranques sostenidos.

---

## Descartado / fuera de alcance

- RAMFC todo ceros / `NO_COINCIDE` PRAMIN (artefacto de lectura; PBDMA consumió GPFIFO).
- rtl8169 DOWN; iGPU AMD `1002:1638`.

Reflash lo deja el operador: `cargo xtask flash-usb-live /dev/sda --yes --only kernel`.
