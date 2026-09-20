# ROG GA104 + AX200 — xHCI new scheme y arranque estable (20 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias:

- [`target/usb-diagnostic-2026-09-20/`](../target/usb-diagnostic-2026-09-20/) — flush **#42** @ **387 817 ms** (~6,5 min).

Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | flush **#42** |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.3.1** build `07e6ef747-dirty` |
| Kernel en placa (SOSOLOG) | **0.3.1 (`bbd3a343a`)** |
| Hardware | `10de:249c` GA104 + `8086:2723` AX200 + `10ec:8168` DOWN |
| Userspace | **`sosh —`** pid=2; `ask hla` abortado; **`halt`** |
| USB stick | `048d:1234` HS — **timeout EP0 5 s** tras Address Device; luego OK |
| USB HID | `0b05:1866`, **`0b05:18c6`** enumeran (sin GET config timeout) |
| WiFi | ALIVE; scan **25** BSS; 4-way; DHCP **192.168.68.132/24** LxWifi |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado`; sin `matvec-res` |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — `drivers/usb/core/hub.c` (`hub_port_init`, new scheme), `drivers/usb/host/xhci.c` (`SETUP_CONTEXT_ONLY` + `TRB_BSR`) |

Hostchecks (host, sesión de implementación):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK**
- `./scripts/l6-g3-gsp-hostcheck.sh` — **OK**
- `cargo test -p xhci-nostd` — **OK** (BSR + política new scheme)
- `cargo xtask test-usb` — **OK**

Matriz: [`hw-matrix.json`](hw-matrix.json) — `ga107-igpu`, `ax200-wifi`; **sin** `record-boot`.

---

## Tabla de etapas (flush #42)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.3.1 (bbd3a343a)` → `sosh —` pid=2 | **OK** |
| USB BOT | `048d:1234` timeout EP0 → `EP recovered` → descriptor OK | **fail** (retraso ~5 s; fix en árbol) |
| USB HID | `0b05:18c6` keyboard ready | **OK** |
| GPU GSP / pool / CE | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado` | **OK** |
| GPU compute / carga | `ask: cargando…` sin `matvec-res` / `on_gpu` | **pendiente** |
| WiFi ALIVE / init | `UCODE_ALIVE_NTFY`; `INIT_COMPLETE_NOTIF` | **OK** |
| Scan | `scan fin count=25` | **OK** |
| 4-way / DHCP | `192.168.68.132/24` backend `lx-wifi` | **OK** |
| Reconexión | no ejercitada | **pendiente** |
| Apagado | `$ halt` → `apagando…` | **OK** |

---

## Hallazgos

### 1. Timeout EP0 en stick live `048d:1234` (confirmado)

**Síntoma:** `transfer event timeout after 5000 ms slot=1 dci=1` inmediatamente después de `slot 1 addressed successfully`; tras `EP recovered` el descriptor de dispositivo llega bien.

**soso (antes del fix):** [`crates/xhci-nostd/src/driver.rs`](../crates/xhci-nostd/src/driver.rs) — `address_device` siempre `BSR=0`, luego GET 8 B (esquema viejo).

**Linux 6.6.32:** [`hub.c:4940-5034`](../lxdde/linux/drivers/usb/core/hub.c) — new scheme: `xhci_enable_device` (`SETUP_CONTEXT_ONLY`, BSR=1) → GET **64 B** en addr 0 → reset → SET_ADDRESS → `msleep(10)`. [`xhci.c:4211-4214`](../lxdde/linux/drivers/usb/host/xhci.c).

**Cambio en soso (esta sesión):** `usb2_new_scheme_address`: BSR=1 → GET 64 → evaluate EP0 → reset puerto → Address Device BSR=0; fallback al esquema viejo si falla.

**Validación host:** `cargo test -p xhci-nostd`, `cargo xtask test-usb` OK. **Placa:** tras `--only kernel`, el log **no** debe mostrar timeout EP0 entre address y `device descriptor: … VID=0x048d`.

### 2. GPU compute / ask (pendiente, no regresión nueva)

**Síntoma:** `ask hla` interrumpido antes de generación; no hay `matvec-res` ni `camino de GPU desactivado` en este flush.

**Estado:** QMD Ampere ya en kernel `bbd3a343a`; validación pendiente de sesión `ask` completa (heredado de [#104](DIAGNOSTICO-ROG-2026-09-18.md)).

### 3. WiFi reconexión (pendiente)

Sin `dhcp perdido` ni recover en este log; `iwl_mvm_ensure_scan_for_connect` ya en árbol desde 18 sep.

---

## Orden de corrección

1. **Flash kernel HEAD** — `cargo xtask flash-usb-live /dev/sda --yes --only kernel` (incluye USB2 new scheme BSR+GET64).
2. **USB stick `048d:1234`** — arranque live; criterio: sin `transfer event timeout … dci=1` antes del device descriptor del BOT.
3. **GPU `ask hola`** — criterio: `matvec-res` con semáforo y `on_gpu=1` sin `camino de GPU desactivado`.
4. **WiFi reconexión** — provocar pérdida DHCP + `wifi connect`; criterio: scan repoblado antes de assoc.
5. **Matriz** — tras evidencia: `parse-logs` + revisión manual; no `record-boot` hasta 3 arranques sostenidos.

---

## Qué no se ha hecho

- Validación en placa del new scheme xHCI (solo host + test-usb QEMU).
- Ciclo `ask` / matvec en silicio con este kernel.
- Reconexión WiFi bajo fallo DHCP.

Reflash lo deja el operador: `cargo xtask flash-usb-live /dev/sda --yes --only kernel`.
