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

---

## Flush #55 — kernel `cfe3c4452` (tarde, 16 sep 2026)

Lectura ESP `/dev/sda1` (`KERNEL`). Copias en
[`target/usb-diagnostic-2026-09-16-run4/`](../target/usb-diagnostic-2026-09-16-run4/).
Connect manual a Rutilo (sin PSK en informe).

| Campo | Valor (flush **#55**, arranque tarde) |
|---|---|
| Kernel en placa (SOSOLOG) | **0.2.2 (`cfe3c4452-dirty`)** |
| Flush | **#55 @ 178451 ms** (~178 s) |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK |
| WiFi | ALIVE + MVM + scan **22 BSS**; **AUTH+ASSOC+4-way OK** (`aid=6`); **sin DHCP** |
| GPU | GSP_INIT_DONE + pool VRAM + apagado limpio |
| Ethernet | rtl8169 enlace DOWN (sin cable); `net: dhcp` al arranque sobre MAC ethernet |

### Tabla de etapas (flush #55)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (cfe3c4452-dirty)` → `sosh —` | **OK** |
| fatlog | flush **#55** @ 178 s; 181413 B (casi lleno) | parcial |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; `GSP-RM dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `init NVM listo` | **OK** |
| Scan userspace | `SCAN_COMPLETE count=22`; Rutilo WPA2 **ch40** | **OK** |
| AUTH / ASSOC | `asociado a 'Rutilo' aid=6 dtim=1 bi=100` | **OK** |
| 4-way EAPOL | `wifi-wpa: 4-way completado, enlace autorizado` | **OK** |
| Beacon TIM | `beacon TIM … tsf=0 gp2=0` × **553** (fatlog saturado) | **FAIL** |
| DHCP LxWifi | sin `net: wifi asociada — solicitando DHCP…` | **FAIL** |

**Bloqueante DHCP:** `net::init()` adjunta **rtl8169** al arranque (`net: backend rtl8169`).
Tras el 4-way, `on_wifi_connected()` llama `attach_now()` (no-op con `NIC_REAL`) y sale en
`if n.backend != Wifi { return; }` — smoltcp sigue en ethernet DOWN; no hay lease WiFi.

**Bloqueante TSF:** `iwl_rx_mpdu_sync_times` solo corría con `assoc_pending_beacon`; tras
`wait_assoc_beacon` el flag se apaga sin leer TSF/GP2 del descriptor v1. Todos los beacons
post-assoc muestran `tsf=0 gp2=0`.

### Corrección aplicada (kernel en árbol, reflash pendiente)

1. **`on_wifi_connected`:** si `wifi_authorized()`, sustituir `NicDev::LxWifi` + MAC iwl,
   `backend=Wifi`, reset DHCP y log `net: wifi asociada — solicitando DHCP…`.
2. **Beacon BSSID:** sync TSF/GP2 en cada beacon (respetar `IWL_RX_MPDU_PHY_TSF_OVERLOAD`;
   fallback TSF en offset 24 del frame 802.11).
3. **`parse_tim_ie`:** log TIM solo al primer beacon o si cambian tsf/dtim (no ×553).

Hostcheck: `./scripts/l6-iwl-fw-hostcheck.sh` **OK** (incl. `test_beacon_sync_descriptor`).

Criterio de validación en placa (tras reflash):

- `net: backend lx-wifi` tras connect; `net: wifi asociada — solicitando DHCP…`.
- `net: dhcp …` con MAC iwl (no confundir con `net: dhcp` del arranque rtl8169).
- Una línea `beacon TIM … tsf≠0` o `gp2≠0`; fatlog sin inundación TIM.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

---

## Flush #52 — kernel `92531cd2d` (16 sep 2026, tarde)

Lectura ESP `/dev/sda1` (`KERNEL`). Copias en
[`target/usb-diagnostic-2026-09-16-run5/`](../target/usb-diagnostic-2026-09-16-run5/).
Connect manual a Rutilo (sin PSK en informe).

| Campo | Valor (flush **#52**) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`92531cd2d`)** |
| Flush | **#52 @ 162722 ms** (~163 s); **173127 B / 256 KiB** (~67 %) |
| Hardware | `10de:249c` (GA104) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2; teclado `sc=76`) |
| WiFi | ALIVE + MVM + scan **24/19 BSS**; **AUTH+ASSOC+4-way+DHCP OK** |
| GPU | GSP_INIT_DONE + pool VRAM + compute sm_86 |
| Ethernet | rtl8169 DOWN (sin cable) |

### Tabla de etapas (flush #52)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (92531cd2d)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#52** @ 163 s; 173127 B (casi lleno) | **FAIL** (ruido RX/TIM) |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; gen2 | **OK** |
| Scan / connect | scan 24 BSS; 4-way; `192.168.68.132/24` LxWifi | **OK** |
| Beacon TIM | `beacon TIM …` × **476** (rate-limit por TSF) | **FAIL** |
| RX debug | `iwl_rx:` × **1628** | **FAIL** |
| SSH | sin sesión en log | pendiente |

**Bloqueante fatlog:** `log_rx()` imprimía cada notif RX (~3/beacon); `parse_tim_ie`
re-logueaba TIM al cambiar TSF/GP2. Linux 6.6.32: `IWL_DEBUG_RX` / `IWL_DEBUG_INFO`
([`iwl-debug.h:181`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-debug.h),
[`mac-ctxt.c:621`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mac-ctxt.c)).

### Corrección aplicada (kernel en árbol, reflash pendiente)

1. **`log_rx`:** silenciar `st=0`; conservar truncado, sin emparejar, SCD-wait.
2. **`parse_tim_ie`:** log TIM solo al primer beacon o cambio DTIM/BI.
3. Hostcheck OK (incl. `test_tim_ratelimit_tsf`).

Criterio placa: sin `iwl_rx:` por beacon; ≤1 línea TIM; DHCP/4-way OK; fatlog &lt;50 % @160 s.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

---

## Flush #42 — kernel `92531cd2d-dirty` (noche, 16 sep 2026)

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-16-run6/`](../target/usb-diagnostic-2026-09-16-run6/).
Sin PSK en este informe (`SOSOWIFI.TXT` vacío; connect manual a Rutilo).

| Campo | Valor (flush **#42**, este arranque) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`92531cd2d-dirty`)** |
| Flush | **#42 @ 452540 ms** (~452 s); **63622 B / 256 KiB** (~24 %) |
| Hardware | `10de:249c` (GA104/GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2; teclado `sc=87` `ent=43`) |
| WiFi | ALIVE + MVM + scan **27 BSS**; **AUTH+ASSOC+4-way+DHCP OK** |
| GPU | GSP_INIT_DONE + pool VRAM + CE readback + compute sm_86 + **apagado `dma=off`** |
| Ethernet | rtl8169 enlace DOWN (sin cable); DHCP real es LxWifi |

Árbol Linux (solo lectura): [`lxdde/linux/`](../lxdde/linux/) **6.6.32**
(`xtask/src/lx_build.rs`). Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK**
(incl. TIM ratelimit) y `./scripts/l6-g3-gsp-hostcheck.sh` **OK**. No cubren
`closed_rb_num` > `IWL_GEN2_RX_N` ni TX/DMA en silicio.

### Tabla de etapas (flush #42)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (92531cd2d-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#42** @ 452 s; 63 KiB (24 %) | **OK** (TIM/RX ya no inundan) |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado`; `GSP-RM dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `up mínimo listo` | **OK** |
| Scan | `SCAN_COMPLETE count=27` | **OK** |
| AUTH / ASSOC | `mlme_auth_ok`; `asociado a 'Rutilo' aid=2 dtim=1` | **OK** |
| 4-way | `wifi-wpa: 4-way completado, enlace autorizado` | **OK** |
| DHCP LxWifi | `net: backend lx-wifi`; `net: dhcp 192.168.68.132/24 gw 192.168.68.1` | **OK** |
| Beacon TIM | 1 línea `tsf=85187808 gp2=85210908` | **OK** |
| RX debug | `iwl_rx:` × **3** (SCD-wait + sin emparejar) | **OK** |
| TX resp | `TX resp` × **99** vs `tx qid=` × **27** (26× `rd=0 wr=0`, 44× `rd=1 wr=1`) | **FAIL** |
| SSH | host key al arranque; sin sesión | pendiente |
| Compute GPU | sin `ask` / matvec en log | pendiente |

### Hallazgos

**1. `drain_rx_gen2` no envuelve `closed_rb` a `queue_size-1` — confirmado**

Síntoma: tras DHCP, 72 `TX resp SUCCESS` duplicados sin doorbell nuevo
(26× `rd=0 wr=0`, luego 44× `rd=1 wr=1`). El anillo RX de soso tiene
`IWL_GEN2_RX_N=32` ([`iwl_internal.h:76`](lxdde/ports/iwlwifi/iwl_internal.h)).
Scan 27 BSS + mgmt + EAPOL superan 32 completados; `rb_stts` llega a ≥32.

soso ([`iwl_trans.c:1036-1054`](lxdde/ports/iwlwifi/iwl_trans.c)):

```
hw = iwl->rb_stts[0] & 0x0fff;          /* 0..4095 */
while (rx_read != hw && n++ < 32) {
    handle_gen2_rx(...);                /* re-parsea páginas viejas */
    rx_read = (rx_read + 1) % 32;       /* 0..31, nunca igual a 32 */
    hw = iwl->rb_stts[0] & 0x0fff;      /* relee sin wrap */
}
```

Linux 6.6.32 ([`pcie/internal.h:193-204`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/internal.h),
[`pcie/rx.c:1513-1517`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/rx.c),
[`rx.c:1567`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/rx.c)):

```
r = iwl_get_closed_rb_stts(...) & 0xFFF;  /* 22000: closed_rb_num */
r &= (rxq->queue_size - 1);               /* 256-1; W/A wrap */
i = (i + 1) & (rxq->queue_size - 1);
```

Compatible con el síntoma: si `hw==32` y `rx_read∈0..31`, el `while` recorre
el anillo entero en cada `drain` (y `iwl_trans_tx` llama `drain_rx_gen2` ×16).
Reinyecta `TX_CMD` (y podría duplicar MPDU). El hostcheck no lo cazó:
`hcmd_queue_test.c` ya escribe `rb_stts[0] = queued % IWL_GEN2_RX_N`.

Misma omisión en [`iwl_trans_8000.c:346-355`](lxdde/ports/iwlwifi/iwl_trans_8000.c)
(no ejercitada en esta placa).

**2. `parse_tx_resp` / doorbell a nivel info — confirmado (ruido)**

soso loguea cada SUCCESS ([`iwl_trans.c:748-759`](lxdde/ports/iwlwifi/iwl_trans.c),
[`1884-1886`](lxdde/ports/iwlwifi/iwl_trans.c)). Linux solo
`IWL_DEBUG_TX_REPLY` / `IWL_DEBUG_TX`
([`mvm/tx.c:1781-1789`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/tx.c),
[`iwl-debug.h:172,182`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-debug.h)).
Tras el wrap del hallazgo 1 el fatlog se llena de SUCCESS; con SSH/datos
pasaría igual aunque el wrap esté bien.

**Descartado como bloqueo:** TIM/RX spam del flush #52 (1 TIM, 3 `iwl_rx`,
fatlog 24 % @452 s). `sku=0x0` en ALIVE AX200 (gen2, no PNVM). rtl8169 DOWN
(sin cable; DHCP no es de esa NIC).

### Orden de corrección

1. **Enmascarar `closed_rb` a `IWL_GEN2_RX_N-1` en `drain_rx_gen2`** (y 8000).
   Archivos: `lxdde/ports/iwlwifi/iwl_trans.c`, `iwl_trans_8000.c`,
   `tools/iwl-hostcheck/rx_datapath_test.c`. Discrepancia Linux `rx.c:1517`.
   Host: `rb_stts=32` y `64` no re-procesan VID ya consumidos.
   Placa: `TX resp` ≈ `tx qid=` tras DHCP; sin 20× el mismo `rd/wr`.

2. **No loguear TX SUCCESS ni cada doorbell.** Archivos: `iwl_trans.c`
   (`parse_tx_resp`, `iwl_trans_tx`). Linux `IWL_DEBUG_TX_REPLY`.
   Host: hostcheck sin cambio de ABI. Placa: fatlog sin ráfaga SUCCESS;
   fallos `0x83` siguen visibles. SSH a `192.168.68.132:22`.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

### Qué no se ha hecho

Ni `drain_rx_gen2` ni el log de TX se han tocado en esta lectura. No hay
ciclo de placa de la corrección. No se ha flasheado. `record-boot` no aplicado.
GPU compute (`ask`) y SSH no se ejercitaron en este arranque.

---

## Flush #39 — kernel `92531cd2d-dirty` (16 sep 2026, 10:22)

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-16/`](../target/usb-diagnostic-2026-09-16/)
(BOOTMARK 10:22). Sin PSK (`SOSOWIFI.TXT` vacío; connect manual a Rutilo).

| Campo | Valor (flush **#39**, este arranque) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`, 2026-09-15T18:13)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`92531cd2d-dirty`)** |
| Flush | **#39 @ 171760 ms** (~172 s); ~51 KiB / 256 KiB |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** pid=2 → **`ip` ENOENT → `init: shell cerrada` → kshell** |
| WiFi | ALIVE + MVM + scan **27 BSS**; **AUTH+ASSOC+4-way+DHCP OK** (`192.168.68.132`) |
| GPU | GSP_INIT_DONE + pool VRAM + CE readback; **sin halt** (`dma=off` no aparece) |
| Ethernet | rtl8169 enlace DOWN (sin cable); lease es LxWifi |

Árbol Linux (solo lectura): [`lxdde/linux/`](../lxdde/linux/) **6.6.32**
(`xtask/src/lx_build.rs`). Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK**
y `./scripts/l6-g3-gsp-hostcheck.sh` **OK**. No cubren spawn/fds ni DMA en silicio.

`closed_rb` wrap (`iwl_closed_rb_idx`) ya está en este kernel sucio:
`TX resp` × **0**, `iwl_rx:` × **6**, un TIM. El hallazgo 1 del flush #42
queda validado en placa en este arranque.

### Tabla de etapas (flush #39)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (92531cd2d-dirty)` → `sosh —` pid=2 | **OK** luego **FAIL** |
| fatlog | flush **#39** @ 172 s; 51 KiB | **OK** |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado (G4e GO)` | **OK** |
| GPU halt | sin `GSP-RM dma=off` (no hubo `halt`) | pendiente |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `up mínimo listo` | **OK** |
| Scan | `SCAN_COMPLETE count=27` (connect) / 25 (autoconnect) | **OK** |
| AUTH / ASSOC | `asociado a 'Rutilo' aid=3 dtim=1 bi=100` | **OK** |
| 4-way | `wifi-wpa: 4-way completado, enlace autorizado` | **OK** |
| DHCP LxWifi | `net: backend lx-wifi`; `net: dhcp 192.168.68.132/24 gw 192.168.68.1` | **OK** |
| TXQ | mgmt tid=15 **antes** de AUTH; data tid=0 **después** del 4-way | **OK** |
| Comando `ip` | `sosh: ip: no existe` | esperado (no hay `/bin/ip`) |
| Shell | prompt `$ ` → `init: shell cerrada; adiós` → `kernel-shell lista` | **FAIL** |
| SSH | host key al arranque; sin sesión (sosh muerta) | pendiente |

Secuencia userspace:

```
$ wifi connect Rutilo …
wifi: asociado a 'Rutilo'
$ net: dhcp 192.168.68.132/24 gw 192.168.68.1
$ ip
sosh: ip: no existe
$ init: shell cerrada; adiós
task: no quedan procesos
kernel-shell lista
```

**Bloqueante:** sosh imprime el error, vuelve a pintar `$ ` y **sale 0**.
init trata el 0 como Ctrl-D / `exit` y no relanza
([`user/init/src/main.rs:388-391`](../user/init/src/main.rs)).

### Hallazgos

**1. `SYS_SPAWN_IO` lee `log_fd` de 8 B que el sosh flasheado no escribe — confirmado**

El kernel sucio añade `SpawnIo.log_fd` (96 B). El rootfs del USB sigue el
manifiesto del 15 sep (`dfec53f84`); el `sosh` empaquetado escribe el layout
de 88 B (HEAD
[`crates/soso-abi/src/lib.rs`](../crates/soso-abi/src/lib.rs) sin `log_fd`).
`sys_spawn_io` hace `user_slice(..., sizeof(SpawnIo))` y el extra es basura
de pila, a menudo **0**.

`build_child_fds` trata `0` como fd del padre, no como centinela, y **mueve**
ese slot:

```
out[slot] = p.fds.get_mut(spec as usize).and_then(|s| s.take());
```

([`kernel/src/task/syscall.rs`](../kernel/src/task/syscall.rs) ~645).
Eso corre **antes** de `vfs::resolve` ([`task/mod.rs`](../kernel/src/task/mod.rs)
`spawn_console_io`). `/bin/ip` no existe → ENOENT → el `Fd::Tty` de stdin
queda en un `stdio_fds` local que se tira. El padre se queda con fd 0 vacío.

Linux 6.6.32 **copia** la tabla, no la mueve:
[`kernel/fork.c:1766-1789`](../lxdde/linux/kernel/fork.c) `copy_files` →
[`fs/file.c:316`](../lxdde/linux/fs/file.c) `dup_fd`. Un `execve` que falla
con ENOENT no cierra el stdin del padre.

Compatible con el síntoma: `sosh: ip: no existe` (ENOENT, no EBADF del spawn)
y acto seguido `read(0)` → EBADF
([`with_fd`](../kernel/src/task/syscall.rs) ~472). `Lector::siguiente` toma
cualquier `n < 0` como EOF
([`user/libsoso/src/linea.rs:112-116`](../user/libsoso/src/linea.rs)) y
`main` devuelve 0 ([`user/sosh/src/main.rs:130-132`](../user/sosh/src/main.rs)).

**2. `take` antes de resolver el ELF, sin restaurar — confirmado (latente)**

Aunque el ABI coincida, un `missingcmd > f` seguiría robando el fd de
redirección si el binario no existe. Linux aplica file actions en el hijo
tras el `fork`; el padre no pierde descriptores si `exec` falla.

**Descartado como bloqueo WiFi/GPU:** 4-way + DHCP LxWifi y GSP/CE en este
mismo log. `rtl8169` DOWN (sin cable). iGPU AMD `1002:1638` sin driver
(fuera de alcance).

### Orden de corrección

1. **No interpretar `log_fd==0` como stdin; clonar o restaurar fds si spawn falla.**
   Archivos: `kernel/src/task/syscall.rs` (`build_child_fds`, `sys_spawn_io`),
   `kernel/src/task/mod.rs` (`spawn_console_io`: resolver ELF **antes** de
   tocar fds), `crates/soso-abi/src/lib.rs`.
   Discrepancia Linux: `dup_fd` copia (`file.c:316`); ENOENT no altera al padre.
   Host: spawn de un path inexistente; el padre sigue leyendo fd 0; layout
   viejo de `SpawnIo` (88 B + ceros) no roba stdin.
   Placa: `ip` → `no existe` → prompt vivo; sin `init: shell cerrada`.

2. **EOF de tty ≠ error de lectura.** Archivos: `user/libsoso/src/linea.rs`,
   `user/sosh/src/main.rs`. Ctrl-D en línea vacía sigue saliendo 0; EBADF/EIO
   imprimen el errno y salen ≠ 0 para que init relance
   ([`user/init/src/main.rs:393-397`](../user/init/src/main.rs)).
   Host: suite init / test de `Lector` con read&lt;0.
   Placa: si volviera a perderse stdin, nueva sosh en vez de kshell.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

(Si el rootfs no se actualiza, el ítem 1 tiene que tolerar el `SpawnIo` de 88 B.)

### Qué no se ha hecho

Ni `build_child_fds` ni `Lector` se han tocado en esta lectura. No hay ciclo
de placa de la corrección. No se ha flasheado. `record-boot` no aplicado.
SSH y `ask` no se ejercitaron: la shell murió antes.
