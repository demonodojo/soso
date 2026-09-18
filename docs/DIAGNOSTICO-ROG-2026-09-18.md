# ROG GA104 + AX200 — QMD Ampere y WiFi post-recover (18 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-18/`](../target/usb-diagnostic-2026-09-18/).
Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | Valor (flush **#104**, último arranque en el log) |
|---|---|
| Kernel en placa (SOSOLOG) | **0.2.2 (`4f94c6cb9-dirty`)** |
| Flush / uptime | **#104 @ 19 797 566 ms** (~5,5 h); ~96 KiB |
| Hardware | `10de:249c` (GA104 Ampere 16 GiB) + `8086:2723` (AX200) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** pid=2; **`halt`** |
| WiFi (boot) | ALIVE + 4-way + DHCP LxWifi **192.168.68.132/24** |
| WiFi (recon) | `net: dhcp perdido` → recover → **`0 BSS`** → `wifi connect` E/S; `TXQ data (EAPOL) falló` |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; **CE subida 435/435** (2212 MiB); **matvec-res** QMD timeout → CPU |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — iwlwifi restart/rescan, nouveau GSP |
| OGKM 570.144 | `cla0c0qmd.h` (QMDV01_07), `clc7c0.h` (`SET_QMD_VERSION` 0x0288) |

Hostchecks (host, tras parches de este informe):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK**
- `./scripts/l6-g3-gsp-hostcheck.sh` — **OK**

No acreditan silicio tras reflash; el USB analizado llevaba **`4f94c6cb9-dirty`** sin estos cambios.

Matriz: [`hw-matrix.json`](hw-matrix.json) — entradas `ga107-igpu`, `ax200-wifi` vía `parse-logs` + ajuste manual. **Sin** `record-boot`.

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

**Cambio en soso:** `QMD_VERSION=7` en `gsp_compute_fill_qmd_v02_grid`; encoder emite **0x01070107**; hostcheck R5.3 + banco exigen minor=7 y PB de **40 B** (10 dwords).

**Avance vs [17 sep](DIAGNOSTICO-ROG-2026-09-17.md):** el hang CE G6 (`+64` / 1 PTE) **ya no aparece**; el bloqueo pasó al **primer QMD**.

### 2. Scan UMAC tras recover (bloqueante reconexión WiFi)

**Síntoma:** tras `timeout cmd … SCAN_REQ`, recover ALIVE pero **`scan_count=0`** al conectar.

**Cambio:** `iwl_trans_recover` limpia lista de BSS; `iwl_mvm_ensure_scan_for_connect` llama `iwl_mvm_scan` si `scan_count==0` antes de `pick_bss` (open y WPA2).

### 3. Parser matriz A8

**Síntoma:** `44713 matvec` marcaba compute/carga **ok** con `on_gpu=0` y `camino de GPU desactivado`.

**Cambio:** `gpu_compute_fallido` reconoce esas cadenas; `carga_real` exige `on_gpu=1` sin fallo GPU; fixture `xtask/fixtures/rog-2026-09-18-gpu-cpu.log`.

---

## Descartado / fuera de alcance

- RAMFC todo ceros / `NO_COINCIDE` PRAMIN (artefacto de lectura; PBDMA consumió GPFIFO).
- rtl8169 DOWN; iGPU AMD `1002:1638`.

Reflash lo deja el operador: `cargo xtask flash-usb-live /dev/sda --yes --only kernel`.
