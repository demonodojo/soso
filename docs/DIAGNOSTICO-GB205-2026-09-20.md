# GB205 + AX211 — LINK_CONFIG y CE embed (20 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-20/`](../target/usb-diagnostic-2026-09-20/) — flush **#55** @ **574 610 ms** (~9,6 min).

Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | Valor |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.3.1** build `07e6ef747-dirty` |
| Kernel en placa (SOSOLOG) | **0.3.1 (`750e9026e`)** |
| Hardware | `10de:2f18` GB205 + `8086:7f70` AX211 + `10ec:8125` sin driver |
| Userspace | **`sosh —`** pid=2; `wifi scan` E/S; `ask` embed falló |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; CE readback OK; **CE atascado** en subida embed |
| WiFi | ALIVE + PNVM + INIT OK; **timeout `grp=3 id=0x09`** en LINK MODIFY active |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — [`mvm/link.c`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/link.c), [`mvm/mld-mac80211.c`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mld-mac80211.c), [`fw/api/mac-cfg.h`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/mac-cfg.h) |
| [`lxdde/reference/linux-master-nouveau/`](../lxdde/reference/linux-master-nouveau/) | **fc02acf** — [`nouveau_boa0b5.c`](../lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nouveau_boa0b5.c) |
| [`lxdde/reference/open-gpu-kernel-modules-570.144/`](../lxdde/reference/open-gpu-kernel-modules-570.144/) | **570.144** — `NVC6B5_LINE_COUNT` |

Hostchecks (sesión de implementación):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK** (incl. secuencia LINK bind/activate)
- `./scripts/l6-g3-gsp-hostcheck.sh` — **OK** (incl. DMA troceado 96 KiB / +64)

Matriz: [`hw-matrix.json`](hw-matrix.json) — `gb205-dgpu`, `ax211-wifi`; **sin** `record-boot`.

---

## Tabla de etapas (flush #55)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.3.1 (750e9026e)` → `sosh —` pid=2 | **OK** |
| USB live | `048d:1234` BOT OK; HID `1462:1603` | **OK** |
| GPU GSP / pool / CE bring-up | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado` | **OK** |
| GPU ask / embed | CE semáforo 14/15; `CE atascado`; `tensor embed gpu=subida de pesos` | **FAIL** |
| WiFi ALIVE / init | `UCODE_ALIVE_NTFY`; `INIT_COMPLETE_NOTIF` | **OK** |
| WiFi MLD up | `MAC_CONFIG scan id=0 ok`; `LINK_CONFIG ADD ok`; timeout `id=0x09` | **FAIL** |
| Scan / assoc | `wifi scan: error de E/S`; `fallo AUTH+ASSOC` | **FAIL** |
| RTL8125 | `RED SIN DRIVER 83:00.0 10ec:8125` | etapa ausente |
| Apagado | `$ halt` → `apagando…` | **OK** |

---

## Hallazgos

### 1. LINK_CONFIG MODIFY active sin atar PHY (confirmado vs Linux 6.6)

**Síntoma:** Tras `MAC_CONFIG` y `LINK_CONFIG ADD`, `timeout cmd grp=3 id=0x09` y `LINK_CONFIG MODIFY active falló` (×4 recuperaciones).

**soso (antes del fix):** [`iwl_mvm_up.c`](../lxdde/ports/iwlwifi/iwl_mvm_up.c) — un solo MODIFY con `ACTIVE` y `phy_id=0`, sin rates.

**Linux:** [`mld-mac80211.c:289-322`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mld-mac80211.c) — primero `iwl_mvm_link_changed(..., 0, false)` con PHY asignado; luego `MODIFY_ACTIVE | MODIFY_RATES_INFO`. [`mac-cfg.h:429-431`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/mac-cfg.h): **`phy_id` ignorado si el link pasa a activo**.

**Fix en árbol:** MODIFY bind (`active=0`, `modify_mask=0`, `phy_id=0`) + MODIFY activate con `RATES_INFO` y `cck=0x0f` / `ofdm=0xff`. Host: [`mld_mac_route_test.c`](../tools/iwl-hostcheck/mld_mac_route_test.c).

### 2. CE DMA monolítico en embed (~5,6 MiB, origen +64) (confirmado; fix aplicado)

**Síntoma:** Bring-up CE OK; en `ask` copia #15 (5 898 240 B, origen +64) no señaliza; `CE atascado`; offload GPU desactivado.

**soso (antes):** [`gsp_buf_upload_dma`](../lxdde/ports/nouveau/gsp_buf.c) — un `gsp_ce_copy_sync` por lote completo.

**Referencia:** Linux `nve0_bo_move_copy` trocea por páginas; el rebote [`gsp_buf_upload_at`](../lxdde/ports/nouveau/gsp_buf.c) ya troceaba a `scratch_bytes`.

**Fix:** bucle de copias ≤ `scratch_bytes` (1 MiB en producción); drenado RPC ampliado en [`gsp_ce_drain_events`](../lxdde/ports/nouveau/gsp_ce.c). Host: troceo 96 KiB / +64 en `gsp-hostcheck`.

---

## Orden de corrección (implementado en árbol)

1. **LINK bind + activate** — [`iwl_mvm_up.c`](../lxdde/ports/iwlwifi/iwl_mvm_up.c), [`iwl_internal.h`](../lxdde/ports/iwlwifi/iwl_internal.h) (`LINK_CONTEXT_MODIFY_RATES_INFO`); validación [`mld_mac_route_test.c`](../tools/iwl-hostcheck/mld_mac_route_test.c).
2. **DMA CE troceado** — [`gsp_buf.c`](../lxdde/ports/nouveau/gsp_buf.c), [`gsp_ce.c`](../lxdde/ports/nouveau/gsp_ce.c); validación [`tools/gsp-hostcheck/main.c`](../tools/gsp-hostcheck/main.c).

**Placa:** `cargo xtask flash-usb-live /dev/sda --yes --only kernel` — esperar `LINK_CONFIG active link=0 phy=0 ok`, `wifi scan` con `count>0`, y `ask` sin `CE atascado` en embed.

---

## Qué no se ha hecho

- Validación silicio post-reflash de los fixes.
- Driver **RTL8125** (`10ec:8125`).
- Reflash desde el agente.
