# ROG AX200: regresión AUTH (TXQ data pre-AUTH) — 16 sep 2026

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-16/`. Sin PSK en este informe (`SOSOWIFI.TXT`
vacío; connect manual).

| Campo | Valor (flush **#26**, último arranque) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`d12188944`)** |
| Fix EAPOL en kernel flasheado | **`ffb7692c1`** (cola data + offload) |
| Flush | **#26 @ 144675 ms** (~145 s) |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2) |
| WiFi | ALIVE + MVM + scan **26 BSS**; **AUTH timeout** (sin TX resp) |
| GPU | GSP_INIT_DONE + pool VRAM + compute sm_86 + apagado limpio |
| Ethernet | rtl8169 enlace DOWN (sin cable) |

Comparativa A/B:

| Kernel | TXQ data tid=0 en assoc_prepare | AUTH / ASSOC |
|---|---|---|
| **`6f558f072`** (flush #50) | **no** | **OK** |
| **`d12188944`** (flush #26) | **sí** (regresión) | **FAIL** (timeout) |

Árbol Linux (solo lectura): [`lxdde/linux/`](../lxdde/linux/) **6.6.32** —
`iwl_mvm_sta_alloc_queue_tvqm` reserva tid=0 solo al primer stream de datos;
AUTH/ASSOC usan cola mgmt (`IWL_MGMT_TID=15`).

Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK** tras el fix en árbol
(assoc_prepare: 1 SCD mgmt; EAPOL lazy tid=0 + `offload_assist=0x0c00` +
`IWL_TX_FLAGS_CMD_RATE`). No acredita TX/DMA en silicio.

---

## Tabla de etapas (flush #26 — kernel `d12188944`)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (d12188944)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#26** @ 144 s | OK |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; compute sm_86; `GSP-RM dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `up mínimo listo` | **OK** |
| Scan userspace | `SCAN_COMPLETE count=26`; Rutilo WPA2 **ch40** | **OK** |
| Connect | `wifi connect Rutilo …` | parcial |
| ADD_STA | `ADD_STA status=0x00000001` | **OK** |
| TLC / SCD / TXQ | `TXQ mgmt qid=1 tid=15` → **`TXQ data qid=2 tid=0`** | **regresión** |
| SESSION_PROT | `SESSION_PROTECTION CONF_ASSOC ok` | **OK** |
| AUTH | `tx qid=1 len=50` → un RX `0xc1` → **sin TX_CMD `0x1c`** | **FAIL** |
| AUTH retry | `AUTH sin retry ctl-filter (sin TX resp)` → corte inmediato | **FAIL** |
| ASSOC / 4-way / DHCP | sin `mlme_auth_ok`; sin lease LxWifi | **FAIL** / pendiente |

Secuencia:

```
… → ADD_STA + TLC + SCD mgmt tid=15
→ SCD data tid=0 + TXQ data qid=2   ← antes del AUTH (Linux no lo hace)
→ SESSION_PROTECTION CONF_ASSOC ok
→ tx qid=1 doorbell len=50 (AUTH)
→ un RX 0xc1, cero TX resp 0x1c
→ AUTH sin retry (sin TX resp) → AUTH timeout
```

**Bloqueante:** reservar la cola TVQM **tid=0** en `assoc_prepare` impide que el
FW confirme el AUTH por la cola mgmt. El fix EAPOL (`ffb7692c1`) no se ejercita
porque no se llega a ASSOC.

---

## Corrección aplicada (kernel en árbol, reflash pendiente)

1. **Aplazar TXQ data:** quitar `iwl_trans_txq_alloc_data` de `iwl_mvm_assoc_prepare`;
   reservar tid=0 en `iwl_mvm_tx_8023` al primer EAPOL (802.3), ya asociado.
   Sin fallback a cola mgmt (evita el `0x83` del flush #50).

2. **AUTH retry:** no abortar el 2.º intento con `IN_CONTROL_AND_MGMT` cuando
   `last_mgmt_tx_status==0` (sin TX resp).

3. **EAPOL:** mantener `offload_assist=0x0c00` + `IWL_TX_FLAGS_CMD_RATE` en cola data.

Criterio de validación en placa (tras reflash):

- Sin `TXQ data` hasta el primer EAPOL.
- AUTH/ASSOC: `TX resp status=0x01`, `mlme_auth_ok`, `asociado a 'Rutilo'`.
- EAPOL: `tx qid=2 len=173` → `status=0x01` (no `0x83`) → `wifi-wpa: 4-way completado`.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

---

## Referencias

- Diagnóstico previo EAPOL `0x83`: [`DIAGNOSTICO-ROG-2026-09-15.md`](DIAGNOSTICO-ROG-2026-09-15.md) (flush #50, `6f558f072`)
- Matriz: [`hw-matrix.json`](hw-matrix.json) — `ax200-wifi`, `ga107-igpu` (sin `record-boot`)

---

## Flush #34 — kernel `cfe3c4452` (tarde, 16 sep 2026)

Lectura ESP `/dev/sda1` (`KERNEL`). Copias en
[`target/usb-diagnostic-2026-09-16-run2/`](../target/usb-diagnostic-2026-09-16-run2/).
Sin PSK en este informe (`SOSOWIFI.TXT` vacío; connect manual a Rutilo).

| Campo | Valor (flush **#34**, último arranque) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`cfe3c4452`)** |
| Fix AUTH (aplazar TXQ data) | **sí** — sin `TXQ data` pre-AUTH |
| Flush | **#34 @ 154347 ms** (~154 s) |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK |
| WiFi | ALIVE + MVM + scan **25 BSS**; AUTH+ASSOC TX OK; **MAC_CONTEXT `is_assoc=1` timeout** |
| GPU | GSP_INIT_DONE + pool VRAM + CE readback + apagado limpio (`dma=off`) |
| Ethernet | rtl8169 enlace DOWN (sin cable) |

Comparativa frente a flush #26 (`d12188944`):

| Kernel | AUTH | ASSOC | MAC_CONTEXT |
|---|---|---|---|
| **`d12188944`** (flush #26) | **FAIL** (TXQ data pre-AUTH) | — | — |
| **`cfe3c4452`** (flush #34) | **OK** (`mlme_auth_ok`) | TX OK (sin log `rx ASSOC`) | **FAIL** (`timeout cmd id=0x28`) |

### Tabla de etapas (flush #34)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (cfe3c4452)` → `sosh —` | **OK** |
| fatlog | flush **#34** @ 154 s | OK |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; `GSP-RM dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `init NVM listo` | **OK** |
| Scan userspace | `SCAN_COMPLETE count=25`; Rutilo WPA2 **ch3** | **OK** |
| Connect | `wifi connect Rutilo …` | parcial |
| ADD_STA | `ADD_STA status=0x00000001` | **OK** |
| TLC / SCD / TXQ | `TXQ mgmt qid=1 tid=15` — **sin TXQ data pre-AUTH** | **OK** |
| SESSION_PROT | `SESSION_PROTECTION CONF_ASSOC ok` | **OK** |
| AUTH | `tx qid=1 len=50` → `TX resp status=0x01` → `mlme_auth_ok` | **OK** |
| ASSOC | `tx qid=1 len=88` → `TX resp status=0x01` | parcial (sin `rx ASSOC` en log) |
| MAC is_assoc=1 | `timeout cmd grp=1 id=0x28 seq=0x0017` → `MAC_CONTEXT is_assoc=1 falló` | **FAIL** |
| 4-way / DHCP | sin EAPOL (MVM parado) | **FAIL** / pendiente |

Secuencia:

```
… → ADD_STA + TLC + SCD mgmt tid=15 (sin cola data)
→ SESSION_PROTECTION CONF_ASSOC ok
→ AUTH TX resp 0x01 → mlme_auth_ok
→ ASSOC TX resp 0x01
→ MAC_CONTEXT is_assoc=1 (dtim/TBTT a cero) → timeout id=0x28
```

**Bloqueante:** soso mandaba `MAC_CONTEXT is_assoc=1` inmediatamente tras ASSOC con
`dtim_period=1` por defecto y campos TBTT (`dtim_tsf`, `dtim_time`,
`assoc_beacon_arrive_time`) a cero. Linux 6.6.32 solo pone `is_assoc=1` tras beacon
real + `iwl_mvm_set_fw_dtim_tbtt`.

### Corrección aplicada (kernel en árbol, reflash pendiente)

1. Tras AUTH+ASSOC: `MAC_CONTEXT is_assoc=0` + `IN_BEACON` (sin `dtim_period` por defecto).
2. Esperar beacon del BSSID; capturar TSF/GP2 del descriptor RX v1; portar
   `iwl_mvm_set_fw_dtim_tbtt`; entonces `is_assoc=1`.
3. Log `rx ASSOC status/aid`.
4. Hostcheck: `./scripts/l6-iwl-fw-hostcheck.sh` **OK**.

Criterio de validación en placa (tras reflash):

- Sin timeout `id=0x28`; log `asociado a 'Rutilo' aid=… dtim=…`.
- EAPOL lazy tid=0 (ya en árbol): `tx qid=2 len=173` → `status=0x01` →
  `wifi-wpa: 4-way completado`.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

---

## Flush #33 — kernel `5e48a396c` (mañana, 16 sep 2026)

Lectura ESP `/dev/sda1` (`KERNEL`). Copias en
[`target/usb-diagnostic-2026-09-16-run3/`](../target/usb-diagnostic-2026-09-16-run3/).
Connect manual a Rutilo (sin PSK en informe).

| Campo | Valor (flush **#33**, arranque matutino) |
|---|---|
| Kernel en placa (SOSOLOG) | **0.2.2 (`5e48a396c-dirty`)** |
| Flush | **#33 @ 132633 ms** (~133 s) |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK |
| WiFi | ALIVE + MVM + scan **22 BSS**; connect → **SESSION_PROTECTION timeout** |
| GPU | GSP_INIT_DONE + pool VRAM + compute sm_86 + apagado limpio |

### Tabla de etapas (flush #33)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (5e48a396c-dirty)` → `sosh —` | **OK** |
| fatlog | flush **#33** @ 133 s | OK |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; compute sm_86; `GSP-RM dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `init NVM listo` | **OK** |
| Scan userspace | `SCAN_COMPLETE count=22`; Rutilo WPA2 **ch3** | **OK** |
| Connect | `wifi connect Rutilo …` | parcial |
| SESSION_PROT | `timeout cmd grp=3 id=0x05 slot=18` → `SESSION_PROTECTION assoc falló` | **FAIL** |
| AUTH / 4-way / DHCP | sin AUTH (MVM parado) | **FAIL** / pendiente |

**Bloqueante:** kernel `5e48a396c` manda `SESSION_PROTECTION` justo tras `ADD_STA`, sin
reservar cola mgmt TLC/SCD/TXQ (HEAD `cfe3c4452` ya lo corrige). No reabrir esa vía.

### Corrección MAC_CONTEXT DTIM/TBTT (16 sep tarde, en árbol)

1. **`iwl_mvm_mac_context_assoc`:** `bi`/`dtim_interval`/`listen_interval`/`assoc_id`
   siempre (Linux `mac-ctxt.c:706–711`); `is_assoc=1` solo con `dtim_period` + TBTT.
2. **Beacon/TIM:** `sync_beacon_seen` al parsear TIM (no exige TSF/GP2); log TSF/GP2/dtim.
3. **Hostcheck:** `assoc_abi_test` + `l6-iwl-fw-hostcheck.sh` OK.

Criterio de validación en placa (tras reflash con HEAD):

- Sin `SESSION_PROTECTION` timeout (kernel reciente con TXQ mgmt previo).
- Sin `MAC_CONTEXT is_assoc=1` timeout; log `asociado a 'Rutilo' aid=… dtim=…`.
- EAPOL lazy tid=0 → `wifi-wpa: 4-way completado`.
