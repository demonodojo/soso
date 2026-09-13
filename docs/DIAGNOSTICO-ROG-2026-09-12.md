# ROG: GPU GA107 y WiFi AX200 — diagnóstico (12 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`KERNEL`, vfat 192 MiB) con `udisksctl`. Copias:

| Run | Hora | Directorio |
|---|---|---|
| run2 | 12:47 | `target/usb-diagnostic-2026-09-12-run2/` |
| run3 | 13:32 | `target/usb-diagnostic-2026-09-12-run3/` |
| validación ABI+flush | ~20:57 | `target/usb-diagnostic-2026-09-12/` (flush #21) |
| run4 | 22:13 | `target/usb-diagnostic-2026-09-12-run4/` |
| **run5 (esta lectura)** | 22:27 BOOTMARK / flush #39 @ 162 s | `target/usb-diagnostic-2026-09-12-run5/` |

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`29056871b-dirty`)** |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200) + `10ec:8168` (rtl8169 VER_46) |
| Userspace | **`sosh —`** + marca OTA `pid=2` `write=165ms` |
| SOSOWIFI | vacío (0 B). Autoconnect a `soso-open` (ausente). Luego `wifi connect Rutilo` a mano. |
| BOOTMARK | UEFI AMI 2.70; shim escribió a las 22:27. SOSOLOG **nuevo**: flush #39 @ 162309 ms. |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** (`3b88125cb`) — `mvm/mac-ctxt.c`, `mvm/phy-ctxt.c`, `mvm/rxmq.c`, `mvm/sta.c`, `fw/api/{mac,sta,rx,commands}.h`, `r8169_phy_config.c`, `r8169_main.c` |
| `lxdde/reference/linux-master-nouveau/` | **`fc02acf`** |
| OGKM | **570.144** |

Hostchecks (22:33, host; no cubren DMA/silicio ni scan/assoc real): `l6-iwl-fw-hostcheck.sh` OK (SCAN_REQ v15 1940 B; `assoc_abi_test` **exige** `is_assoc=1`, que es lo contrario de Linux), `l6-g3-gsp-hostcheck.sh` OK, `l6-rtl-fw-hostcheck.sh` OK (`rtl8168h-2.fw` 211 opcodes + cola en `rtl8169.rs`).

---

## Tabla de etapas (run5 = flush #39 @ 162 s)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, marca OTA `pid=2` `write=165ms` | **OK** |
| fatlog | flush **#39** @ 162309 ms, `kbd sc=139` | OK |
| GSP / RPC | `GSP_INIT_DONE res=0x0`, `GSP booted (hw, booter_load Ampere + RPC, 16384 MiB VRAM)` | **OK** |
| CE / pool | `CE selftest OK`, `pool VRAM=sí`, sondas CE mueven | **OK** |
| Golden GR + compute | `compute listo cls=0xc7c0` | **OK** |
| WiFi alive/init/MVM | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, BINDING/POWER/`SCAN_CFG v5 ok` | **OK** |
| WiFi SCAN | `SCAN_REQ_UMAC v15 1940 B` → `scan fin count=21` (luego 23 y 18) | **OK** (canal/RSSI mal parseados) |
| WiFi assoc | `wifi connect Rutilo` → `'Rutilo' … canal 3; falta el 4-way (R8)` → **`timeout cmd grp=1 id=0x28 slot=16; MVM parado`** / `MAC_CONTEXT falló` | **FAIL** |
| Ethernet | `firmware PHY cargado (… 211 opcodes)` + `PHY 8168H config Linux (ioffset=0x5898 rlen=5)`; `phystatus 0x84 bmsr 0x7989` → **DOWN 10M half** | **FAIL** (MAC start 8168h ausente; cable no excluido) |
| DHCP | `net: dhcp…` sin lease | **pendiente** (no marcar `ok`) |
| ask 27B | `$ ask hola` → `cargando Qwen3.8-27B…`; sin matvec / PF | **sin evidencia** |
| rootfs write | `hw-inv: inventario en /etc/soso-hw` | **OK** |

**No `--boot-ok` / no racha:** sin lease, assoc WPA2 falló, MVM parado.

---

## Hallazgos

### WiFi-1. `MAC_CONTEXT` MODIFY con `is_assoc=1` sin DTIM — timeout y cola envenenada (confirmado)

**Síntoma:** tras `wifi connect Rutilo`, `iwl_trans: timeout cmd grp=1 id=0x28 slot=16; MVM parado` y `iwl_mvm: MAC_CONTEXT falló`. El slot queda `POISON`; no hay más HCMD.

**soso:** [`iwl_mvm_assoc.c:28–45`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c) `iwl_mvm_mac_context_assoc`: `action=MODIFY`, `is_assoc=1`, `dtim_interval`/`bi`/`dtim_time` a cero. [`wifi_wpa.rs:69`](kernel/src/net/wifi_wpa.rs) llama a `connect_wpa2` del driver **antes** del 4-way; el driver hace ese MAC_CONTEXT como primer HCMD.

**Linux 6.6.32:** [`mvm/mac-ctxt.c:680–698`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mac-ctxt.c): *«We need the dtim_period to set the MAC as associated»*. `is_assoc=1` solo si `vif->cfg.assoc && vif->bss_conf.dtim_period`. Si no, `is_assoc=0` y `MAC_FILTER_IN_BEACON`. `dtim_interval = beacon_int * dtim_period` (L707).

**Compatibilidad síntoma:** el ADD del boot (`MAC_CONTEXT scan id=0 ok`, `is_assoc=0`) contestó. El MODIFY con `is_assoc=1` y DTIM=0 no; el FW no responde y soso envenena la cola. El hostcheck `assoc_abi_test.c:123` **exige** `is_assoc=1` — hay que invertirlo.

### WiFi-2. PHY_CONTEXT clavado en ch6; assoc no lo mueve (confirmado, no disparaba hasta run5)

**soso:** [`iwl_mvm_up.c:288`](lxdde/ports/iwlwifi/iwl_mvm_up.c) `PHY_CONTEXT ch6 2.4GHz`. `connect_wpa2` guarda `iwl->channel` y llama `assoc_prepare` **sin** `PHY_CONTEXT_CMD` ni BINDING nuevo.

**Linux:** [`mvm/phy-ctxt.c:285–322`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/phy-ctxt.c) `iwl_mvm_phy_ctxt_changed` (MODIFY, o REMOVE+ADD si cambia de banda CDB).

**Síntoma:** el BSS elegido dice canal 3 (5 GHz listado como 40 en el `wifi scan` de usuario). El radio sigue en ch6. TX/RX de assoc no puede ir al AP. Encaja con un FW sordo al MAC_CONTEXT de estación.

### WiFi-3. Canal del BSS: solo DS Params + RX_PHY mal leído (confirmado)

**Síntoma:** listado `Rutilo: canal 40`; al conectar el kernel dice `canal 3`. Muchos BSS `canal 0`. Todos los RSSI `-70 dBm` (fallback).

**soso:**
- [`iwl_mvm.c:503–584`](lxdde/ports/iwlwifi/iwl_mvm.c): canal = `WLAN_EID_DS_PARAMS` (2,4 GHz). Si falta, `last_rx_channel`. No parsea `WLAN_EID_HT_OPERATION` (61).
- [`iwl_trans.c:303–308`](lxdde/ports/iwlwifi/iwl_trans.c) `parse_rx_phy`: `last_rx_channel = data[1]|(data[2]<<8)` (= `cfg_phy_cnt`/`stat_id`).
- [`iwl_trans.c:326–345`](lxdde/ports/iwlwifi/iwl_trans.c) `parse_rx_mpdu`: salta el desc y **no** lee `channel` ni energía.

**Linux 6.6.32:**
- Canal del beacon: [`net/wireless/scan.c:1920–1929`](lxdde/linux/net/wireless/scan.c) DS Params **o** `HT_OPERATION.primary_chan`.
- AX200 (`device_family < AX210`): [`mvm/rxmq.c:2361`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/rxmq.c) `phy_data.channel = desc->v1.channel`.
- RX_PHY: [`fw/api/rx.h:49–58`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/rx.h) `channel` es `__le16` en offset **22**, no en bytes 1–2.

**Compatibilidad:** 5 GHz no lleva DS Params → canal basura (3) vs 40 del listado. RSSI `-70` = `last_rx_rssi==0`.

### WiFi-4. `ADD_STA_KEY` layout y grupo ≠ Linux (confirmado ABI; no alcanzado en run5)

**soso:** [`iwl_mvm_assoc.c:13–26,105`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c) struct inventada; envío a **`MAC_CONF_GROUP` (0x3)** id `0x17`.

**Linux:** `ADD_STA_KEY = 0x17` en [`enum iwl_legacy_cmds`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/commands.h:128). [`sta.h:361–399`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/sta.h) `common {sta_id, key_offset, key_flags le16, key[32], rx_secur_seq_cnt[16]}` + mic/TSC (~76 B). [`mvm/sta.c:3678`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/sta.c) `iwl_mvm_send_cmd_pdu(mvm, ADD_STA_KEY, …)`.

**Compatibilidad:** el 4-way de [`wifi_wpa.rs`](kernel/src/net/wifi_wpa.rs) no se ejecutó: MLME murió antes. Sigue siendo el siguiente ABI tras arreglar MAC/PHY.

### Eth-1. Firmware PHY + cola `rtl8168h_2` ya en el USB; enlace sigue DOWN (confirmado parcial)

**Síntoma:** `rtl8169: firmware PHY cargado (… 211 opcodes)` y `PHY 8168H config Linux (ioffset=0x5898 rlen=5)` — el ítem 1 del diagnóstico de las 22:13 **está en placa**. Sigue `phystatus 0x84 bmsr 0x7989 TBI → enlace DOWN`. BMSR bit 2 (link) = 0, bit 5 (ANEGCOMPLETE) = 0. `poll_link` 162 s no imprimió UP.

**soso:** [`rtl8169.rs:706–729`](kernel/src/drivers/rtl8169.rs) cola Linux CHIN EST / R-tune / GPHY 10m / ADC / rlen / PFM / 10m PLL / ALDPS / EEE. `phy_autoneg` sí escribe `BMCR_ANENABLE|BMCR_ANRESTART`. **No** está [`rtl_hw_start_8168h_1`](lxdde/linux/drivers/net/ethernet/realtek/r8169_main.c:3288–3347) (EPHY 6 regs, FIFO, ASPM, EEE MAC, OCP `0xd412`/`0xe056`…). Linux no usa `PHYstatus` bit 7 (`TBI_Enable`) para 8168H: el enlace va por phylink/MDIO.

**Compatibilidad:** el fallback EPHY de 6 regs ya no se usa (correcto). Falta el **MAC start** VER_46. `0x84` = `TBI_Enable|_10bps` **sin** `LinkStatus=0x02`. Cable ausente sigue explicando DOWN; no se puede separar hasta `rtl_hw_start_8168h_1` + espera ANEG en placa **con cable**.

### GPU — sin regresión

`GSP_INIT_DONE`, CE, golden GR, `compute listo`, `pool VRAM=sí`. `ask` 27B no deja matvec en fatlog (userspace).

---

## Orden de corrección

| # | Cambio | Archivos | Linux | Hecho host / placa |
|---|---|---|---|---|
| 1 | **`MAC_CONTEXT` assoc: `is_assoc=0`** hasta AUTH+ASSOC y `dtim_period`. No envenenar HCMD. Invertir `assoc_abi_test.c` (hoy exige `is_assoc=1`). | `lxdde/ports/iwlwifi/iwl_mvm_assoc.c`, `tools/iwl-hostcheck/assoc_abi_test.c` | `mvm/mac-ctxt.c:680–698` | Host: primer MODIFY `is_assoc=0` y `IN_BEACON`. Placa: `wifi connect` **no** imprime `timeout cmd … id=0x28`. |
| 2 | **Canal del BSS como Linux:** HT Operation (`primary_chan`) + `desc->v1.channel` del RX_MPDU AX200. RX_PHY: `channel` @ offset 22. | `iwl_mvm.c` `parse_bss`, `iwl_trans.c` `parse_rx_mpdu`/`parse_rx_phy` | `net/wireless/scan.c:1920–1929`, `mvm/rxmq.c:2361`, `fw/api/rx.h:58` | Host: beacon 5 GHz sin DS → canal del HT IE; desc v1. Placa: `Rutilo` deja de salir como canal 3 si el scan lista 40. |
| 3 | **`PHY_CONTEXT_CMD` + BINDING al canal del BSS** (banda 2,4/5) **antes** de MAC MODIFY / ADD_STA. | `iwl_mvm_assoc.c`, `iwl_mvm_up.c` | `mvm/phy-ctxt.c:285` | Host: HCMD PHY_CONTEXT con el canal del beacon. Placa: assoc 5 GHz no se queda en ch6. |
| 4 | **MLME 802.11 AUTH+ASSOC** (o no marcar `associated`). Luego MAC_CONTEXT `is_assoc=1` con DTIM/beacon_int. | `iwl_mvm_assoc.c`, `kernel/src/net/wifi_wpa.rs` | `mac-ctxt.c:680–711` | Host: segundo MODIFY con `dtim_interval != 0`. Placa: `iwl_mvm: asociado` sin timeout. |
| 5 | **`ADD_STA_KEY` ABI Linux v2/v3 + grupo LEGACY 0x17** (no MAC_CONF). Cubrir en `assoc_abi_test.c`. | `iwl_mvm_assoc.c`, `iwl_internal.h`, `assoc_abi_test.c` | `fw/api/sta.h:361–399`, `commands.h:128`, `mvm/sta.c:3678` | Host: `sizeof` + `sta_id`/`key_flags=CCM`. Placa: `wifi-wpa: 4-way handshake completado`. |
| 6 | **`rtl_hw_start_8168h_1`** (MAC EPHY/OCP/ASPM) + esperar ANEG (`BMSR` bit 5) antes de loguear el enlace. No tratar `0x80` como TBI en 8168H. | `kernel/src/drivers/rtl8169.rs` | `r8169_main.c:3288–3347` | Host: opcodes EPHY/OCP de la tabla. Placa: `enlace UP` **con cable**; sin cable, BMSR sigue bit2=0 **después** del start. |

Validación (usuario, **después** de 1–3; SSID real ya se usó en placa):

```bash
# Cable en el RJ45. SOSOWIFI.TXT opcional (ssid= / psk=).
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

---

## Arranque ~20:57 / run4 — histórico

Ítem PHY 8168h_2 **cerrado en run5** (fw 211 opcodes + cola Linux en el USB). SCAN 1940 B / 26 BSS. Assoc no se ejerció (SSID `soso-open` ausente).

## Arranque 13:32 (run3) — histórico

SCAN_REQ 2112 B → timeout 0x0d; `mkdir /tmp` EIO. **Corregido** (1940 B; `hw-inv` escribe).

## Referencia run2 (12:47)

Scan 2112 B y EIO `/tmp`. GPU ya verde.

## Qué no se ha hecho

- No se ha tocado código en esta lectura (solo ESP, Linux, matriz, informe).
- No se ha reflasheado.
- No AUTH/ASSOC 802.11, ni ADD_STA_KEY Linux, ni PHY_CONTEXT de assoc, ni `rtl_hw_start_8168h_1`.
- iGPU AMD `1002:1638` sin driver.
- No `record-boot` ni `--boot-ok`. `parse-logs` subió la racha; **revertido a 11 / 12**. `sesion_sostenida` no es `ok` (el parser vio `sosh` + `net: dhcp`).
