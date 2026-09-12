# ROG: GPU GA107 y WiFi AX200 — diagnóstico (12 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`KERNEL`, vfat 192 MiB) con `udisksctl`. Copias:

| Run | Hora | Directorio |
|---|---|---|
| run2 | 12:47 | `target/usb-diagnostic-2026-09-12-run2/` |
| run3 | 13:32 | `target/usb-diagnostic-2026-09-12-run3/` |
| validación ABI+flush | ~20:57 | `target/usb-diagnostic-2026-09-12/` |
| **run4 (esta lectura)** | 22:13 | `target/usb-diagnostic-2026-09-12-run4/` |

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`29056871b-dirty`)** |
| SOSOHASH (pack 11-sep) | `0870e0bee-dirty` (no se actualiza con `--only kernel`) |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200) + `10ec:8168` (rtl8169) |
| Userspace | **`sosh —`** + marca OTA `pid=2` |
| SOSOWIFI | vacío (0 B) |
| BOOTMARK | UEFI AMI 2.70; mtime 22:10. El SOSOLOG **no** es un arranque nuevo: misma cabecera flush #21 |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `r8169_firmware.c`, `r8169_phy_config.c:796`, `fw/api/sta.h`, `mvm/sta.c`, `mvm/mac-ctxt.c`, `fw/api/commands.h` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** |
| OGKM | **570.144** |

Hostchecks (22:13, host; no cubren DMA/silicio ni scan real): `l6-iwl-fw-hostcheck.sh` OK (SCAN_REQ v15/v17 1940 B, assoc ADD_STA 48 B), `l6-g3-gsp-hostcheck.sh` OK, `l6-rtl-fw-hostcheck.sh` OK (`rtl8168h-2.fw` 211 opcodes en rootfs).

**Nota BOOTMARK 22:10:** el shim escribió marca a las 22:10 y el SOSOLOG sigue siendo flush #21 @ 23396 s (misma sesión que ~20:57). O el USB se reinsertó tras esa sesión, o un arranque 22:10 no llegó a `fatlog`. El análisis usa el volcado de kernel disponible.

---

## Tabla de etapas (run4 = flush #21 @ 23396 s)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, marca OTA `pid=2` `write=167ms` | **OK** |
| fatlog | flush **#21** @ 23396083 ms, `kbd sc=17` | OK |
| GSP / RPC | `GSP_INIT_DONE res=0x0`, `GSP booted (hw, booter_load Ampere + RPC, 16384 MiB VRAM)` | **OK** |
| CE / pool | `CE selftest OK`, `pool VRAM=sí`, sondas CE mueven | **OK** |
| Golden GR + compute | `compute listo cls=0xc7c0` | **OK** |
| WiFi alive/init/MVM | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, BINDING/POWER/`SCAN_CFG v5 ok` | **OK** |
| WiFi SCAN | `SCAN_REQ_UMAC v15 1940 B` → ACK 0x0d → `scan fin count=26` | **OK** |
| WiFi assoc | `'soso-open' no está entre los 26 BSS`; SOSOWIFI vacío; `/etc/wifi.conf` pide `soso-open` | **pendiente** (SSID) + ABI no ejercitada |
| Ethernet | `firmware PHY ausente (/lib/firmware/rtl_nic/rtl8168h-2.fw)`; `phystatus 0x04` `bmsr 0x7989` → **DOWN 10M half** | **FAIL** (fw + PHY vs Linux) |
| DHCP | `net: dhcp…` sin lease | **pendiente** (no marcar `ok`) |
| ask 27B | `$ ask hola` → `cargando Qwen3.8-27B…`; sin matvec / PF | **sin evidencia** |
| rootfs write | `hw-inv: inventario en /etc/soso-hw` | **OK** (flush USB) |

**No `--boot-ok` / no racha:** sin lease, sin assoc, sin matvec en GPU.

---

## Hallazgos

### Eth-1. Firmware PHY 8168H ausente en el USB + config Linux incompleta (confirmado)

**Síntoma:** `rtl8169: firmware PHY ausente (/lib/firmware/rtl_nic/rtl8168h-2.fw)` → `phystatus 0x04` `bmsr 0x7989` → enlace DOWN. Linux `LinkStatus=0x02`; `0x04` es solo `_10bps` **sin** link. BMSR bit 2 (link) = 0.

**soso:** [`kernel/src/drivers/rtl8169.rs`](kernel/src/drivers/rtl8169.rs) `load_phy_firmware` (~L602) y `rtl8168h_hw_phy_config` (~L632): si el fw carga, **return**; si falta, 6 registros EPHY inventados.

**Linux 6.6.32:**
- `r8169_main.c:53` `FIRMWARE_8168H_2="rtl_nic/rtl8168h-2.fw"` para `RTL_GIGA_MAC_VER_46`.
- Intérprete: [`r8169_firmware.c`](lxdde/linux/drivers/net/ethernet/realtek/r8169_firmware.c) (opcodes 0x0–0xe). El de soso **casa** (hostcheck 211 opcodes).
- Tras el fw, [`r8169_phy_config.c:796–833`](lxdde/linux/drivers/net/ethernet/realtek/r8169_phy_config.c) `rtl8168h_2_hw_phy_config`: CHIN EST `0x808a`, R-tune/PGA `0x0811`/`0x0a42`, GPHY 10m, ADC bias `rtl8168h_2_get_adc_bias_ioffset`, rlen TX LPF, disable PFM/10m PLL, ALDPS, EEE. Si el fw falta, `r8169_apply_firmware` es no-op (`tp->rtl_fw` NULL, `r8169_main.c:2190`) y **sigue** esa cola.

**Compatibilidad síntoma:** el USB no tiene el blob (sí está en `rootfs/` del árbol, no en la imagen live). El fallback de 6 EPHY **no existe en Linux**. El resto de `rtl8168h_2_hw_phy_config` no está portado. Cable ausente también explica DOWN; no se puede separar hasta fw+cola Linux en placa.

### WiFi-1. Autoconnect a `soso-open` (confirmado, no es bug de scan)

**Síntoma:** `iwl_mvm: 'soso-open' no está entre los 26 BSS del scan`. Scan **OK** (1940 B, 26 BSS). SOSOWIFI vacío → `/etc/wifi.conf` (`ssid=soso-open`).

**soso:** [`kernel/src/net/wifi_wpa.rs:39`](kernel/src/net/wifi_wpa.rs) + [`iwl_mvm.c:746`](lxdde/ports/iwlwifi/iwl_mvm.c) `iwl_mvm_pick_bss` (SSID exacto, mejor RSSI).

**Linux:** mac80211 no autoconecta a un SSID fantasma. **Confirmado:** assoc no se ejerció. Hace falta SSID real en `SOSOWIFI.TXT`. El log no lista los 26 SSID.

### WiFi-2. PHY_CONTEXT clavado en ch6; assoc no lo mueve (confirmado como etapa ausente)

**soso:** `iwl_mvm_up.c` deja `PHY_CONTEXT ch6 2.4GHz`. `connect_open` guarda `iwl->channel` y llama `assoc_prepare` **sin** `PHY_CONTEXT_CMD` ni BINDING nuevo.

**Linux:** `mvm/phy-ctxt.c` `iwl_mvm_phy_ctxt_changed` al unirse al BSS.

**Síntoma:** no disparado en este log (SSID ausente). Tras un SSID real, TX/RX en el canal del AP fallaría si el PHY sigue en 6.

### WiFi-3. `ADD_STA_KEY` layout y grupo ≠ Linux (confirmado ABI; no en este SOSOLOG)

**soso:** [`iwl_mvm_assoc.c:13–26`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c) struct inventada (`operation`, `encryption_type`, `key_size`…) y envío a **`MAC_CONF_GROUP` (0x3)** id `0x17`.

**Linux 6.6.32:** `fw/api/commands.h` `ADD_STA_KEY = 0x17` en **`enum iwl_legacy_cmds`** (grupo 0 / LONG). Struct [`iwl_mvm_add_sta_key_cmd`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/sta.h:394): `common {sta_id, key_offset, key_flags le16, key[32], rx_secur_seq_cnt[16]}` + `rx_mic`/`tx_mic`/`transmit_seq_cnt` (v2/v3, ~76 B). `mvm/sta.c:3544` `iwl_mvm_send_sta_key`: `STA_KEY_FLG_CCM`, `STA_KEY_FLG_KEYID_*`.

**Compatibilidad:** el 4-way de [`wifi_wpa.rs`](kernel/src/net/wifi_wpa.rs) instalaría una clave que el FW no entiende. Hostcheck `assoc_abi_test.c` **no** cubre ADD_STA_KEY.

### WiFi-4. MLME: `is_assoc=1` sin AUTH/ASSOC 802.11 (etapa ausente)

**soso:** `assoc_prepare` pone `cmd.u.sta.is_assoc = 1` y `iwl->associated = 1` tras ADD_STA, sin tramas de gestión.

**Linux:** `mac-ctxt.c:680–698` `is_assoc=1` solo si `vif->cfg.assoc && dtim_period`. AUTH/ASSOC las manda mac80211.

**ADD_STA v10 48 B** (host) está alineado con `sta.h:326` VER_10. `sta_id=1` vs Linux STA vif que usa id 0 (`sta.c:37–48`); en soso `SCAN_CFG bcast=0`, así que 1 evita chocar con bcast. No es el primer bloqueo.

### GPU — sin regresión

`GSP_INIT_DONE`, CE, golden GR, `compute listo`, `pool VRAM=sí`. `ask` 27B no deja matvec en fatlog (userspace).

---

## Orden de corrección

| # | Cambio | Archivos | Linux | Hecho host / placa |
|---|---|---|---|---|
| 1 | **PHY 8168H como Linux:** no `return` tras fw; portar cola `rtl8168h_2_hw_phy_config` (CHIN EST, R-tune, ADC bias, rlen, ALDPS, EEE). Quitar la tabla EPHY de 6 regs. El blob ya está en `rootfs/lib/firmware/rtl_nic/`. | `kernel/src/drivers/rtl8169.rs`, hostcheck `tools/rtl-hostcheck/phy_fw_test.c` | `r8169_phy_config.c:796–833`, `r8169_main.c:2190` | Host: `firmware PHY cargado (211 opcodes)` + opcodes de la cola. Placa: misma línea y `enlace UP` **con cable**. |
| 2 | **PHY_CONTEXT + BINDING al canal del BSS** antes de ADD_STA. | `lxdde/ports/iwlwifi/iwl_mvm_assoc.c`, `iwl_mvm_up.c` | `mvm/phy-ctxt.c` `iwl_mvm_phy_ctxt_changed` | Host: HCMD PHY_CONTEXT con el canal del beacon. Placa: assoc en 5 GHz no se queda en ch6. |
| 3 | **ADD_STA_KEY ABI Linux v2/v3 + grupo LEGACY 0x17** (no MAC_CONF). Ampliar `assoc_abi_test.c`. | `iwl_mvm_assoc.c`, `iwl_internal.h`, `tools/iwl-hostcheck/assoc_abi_test.c` | `fw/api/sta.h:361–399`, `commands.h:128`, `mvm/sta.c:3544` | Host: `sizeof` + campos `sta_id`/`key_flags=CCM`. Placa: `wifi-wpa: 4-way handshake completado`. |
| 4 | **MLME:** no marcar `is_assoc`/associated hasta AUTH+ASSOC (+ DTIM). TX 802.11 mínimo o documentar el hueco. | `iwl_mvm_assoc.c`, `kernel/src/net/wifi_wpa.rs` | `mac-ctxt.c:680–698` | Host: `is_assoc=0` en el primer MODIFY. Placa: `iwl_mvm: asociado` + lease DHCP. |

Validación (usuario, **después** de 1–3 y SSID real en ESP):

```bash
# SOSOWIFI.TXT: ssid=… y psk=… (o red abierta). Cable en el RJ45.
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes
```

---

## Arranque 13:32 (run3) — histórico

SCAN_REQ 2112 B → timeout 0x0d; `mkdir /tmp` EIO. **Corregido y validado** en flush #21 (1940 B, 26 BSS; `hw-inv` escribe).

## Referencia run2 (12:47)

Scan 2112 B y EIO `/tmp`. GPU ya verde.

## Qué no se ha hecho

- No se ha tocado código en esta lectura (solo ESP, Linux, matriz, informe).
- No se ha reflasheado. El intérprete PHY y ADD_STA v10 están en el árbol host; el USB **no** lleva `rtl8168h-2.fw`.
- No AUTH/ASSOC 802.11, ni ADD_STA_KEY Linux, ni PHY_CONTEXT de assoc.
- iGPU AMD `1002:1638` sin driver.
- No `record-boot` ni `--boot-ok`.
