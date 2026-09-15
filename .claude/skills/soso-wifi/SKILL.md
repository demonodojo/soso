---
name: soso-wifi
description: >-
  soso WiFi Intel (iwlwifi/lxdde) — AX211/AX200, firmware TLV, WPA2 EAPOL,
  VFIO and hostcheck. Use when modifying lxdde/ports/iwlwifi, net/wifi_wpa.rs,
  wificonf, SOSOWIFI.TXT, wifi syscalls, scripts l6-wifi-* / l6-iwl-*, or
  diagnosing scan/assoc/ALIVE on live USB or QEMU passthrough.
---

# soso — WiFi Intel (iwlwifi)

## Plan aplicable y seguimiento

Para implementar o retomar una entrega, leer
[Identificación y seguimiento de planes](../soso-architecture/references/planes.md).
Localizarla en `docs/WIFI-OPERATIVA.md` o el plan señalado por el usuario y
correlacionar el diagnóstico por chip, firmware y build. Si habilita una tarea
de automejora, enlazar el bloqueo con su ID Txx sin cambiar al plan del archivo
abierto en el IDE. Registrar por separado hostcheck, ALIVE, scan, asociación y
tráfico real; pasar una etapa no completa las siguientes. Persistir el próximo
paso y actualizar plan/matriz con sus respectivas evidencias.

Driver **first-party** en `lxdde/ports/iwlwifi/` (no es un port de mac80211
completo). Transporte PCIe + firmware TLV + MVM scan/assoc/TX mínimos.
Supplicant WPA2 en kernel Rust (`kernel/src/net/wifi_wpa.rs`).

## Chips

| PCI ID | Chip | Context-info | Firmware | PNVM |
|--------|------|--------------|----------|------|
| `8086:7f70`, `51f0`, `54f0` | AX211 | gen3 | `iwlwifi-so-a0-gf-a0-{89,77}.ucode` | `iwlwifi-so-a0-gf-a0.pnvm` |
| `8086:2723` | AX200 | gen2 | `iwlwifi-cc-a0-{77,74,73,72,66}.ucode` | no |
| `8086:24fd` | 8265 (familia 8000) | no (FH/ICT) | `iwlwifi-8265-36.ucode` | no |

`iwl->family` (8000 / 22000 / AX210); `gen3` solo si familia AX210. El live
perfil `live-usb` ya incluye `SOSO_LXDDE_MODE=nouveau,iwlwifi`.

Firmware en `rootfs/lib/firmware/`. `lx_request_firmware` prueba alternativas
en silencio; el fallo lo canta el llamante (`iwl_ax211: firmware no encontrado`).

## Archivos

| Path | Rol |
|------|-----|
| `iwl_ax211.c` | probe PCI, carga ucode/pnvm, arranque gen2/gen3/8000 |
| `iwl_fw.c` | parser TLV (`SEC_RT` lmac/umac, paging, pnvm) |
| `iwl_trans.c` | context-info, colas MTR/MCR/RX, espera ALIVE |
| `iwl_trans_8000.c` | carga FH + ICT (8265); plan comprobable en host |
| `iwl_mvm.c` | scan UMAC, assoc, TX |
| `iwl_mvm_data.c` | 802.11 ↔ Ethernet (RX/TX), puro y comprobable en host |
| `iwlwifi_lx.c` | exports C → Rust (`lx_iwlwifi_*`) |
| `lxdde/shim/src/{iwlwifi,skbuff,netdev,cfg80211,mac80211}_lx.c` | shims mínimos |
| `crates/soso-wpa2/` | máquina WPA2-PSK/CCMP `no_std`, con su banco de host |
| `kernel/src/net/wifi_wpa.rs` | sólo E/S: credenciales y bucle del 4-way |
| `kernel/src/drivers/wificonf.rs` | lee `SOSOWIFI.TXT` (ESP, 8.3) |
| `tools/iwl-hostcheck/main.c` | parser TLV contra blobs reales |
| `tools/iwl-hostcheck/rx_datapath_test.c` | descriptores RX por generación + datos + anillo TX |
| `tools/iwl-hostcheck/ring_soak_test.c` | anillos RX/TX contra un modelo de firmware, bajo carga |
| `scripts/l6-wifi-capture-4way.sh` | captura un 4-way real (hwsim + hostapd) como fixture |

## Pila en soso

1. `lxdde` probe → firmware ALIVE (`UCODE_ALIVE_NTFY`). **«ALIVE degradado» no vale.**
2. Syscalls `wifi_scan=56`, `wifi_status=57`, `wifi_connect=58` (`psk_len=0` = abierta).
3. Autoconnect al arranque si `modes.iwlwifi` y ALIVE: `wifi_wpa::autoconnect()`.
4. Credenciales: `SOSOWIFI.TXT` en la ESP (`ssid=` / `psk=`) o `/etc/wifi.conf`.
5. Tras asociación: DHCP **sin** fallback 10.0.2.x. Backend `NicDev::LxWifi`
   (prioridad si ya está asociado; si no, ethernet primero).
6. SSH en placa: puerto **22** (no 2222).

sosh / kshell: `wifi scan|status|connect <ssid> [psk]`.

## Comandos

```bash
cargo xtask lx-build iwlwifi
./scripts/l6-iwl-fw-hostcheck.sh          # parser SEC_RT vs ucode del rootfs (~1 s)
cargo test -p soso-wpa2                   # supplicant WPA2: vectores + transcripciones
sudo ./scripts/l6-wifi-capture-4way.sh    # una vez: 4-way real de hostapd → fixture
sudo ./scripts/l6-wifi-vfio-test.sh       # VFIO AX211 → QEMU; GO = ALIVE real
# Live (nouveau+iwlwifi ya van): editar SOSOWIFI.TXT en ESP p1.
# Agente: monta p1 con udisksctl (skill soso-live, sin sudo/TTY); no uses cargo xtask sosolog.
# El agente no graba el USB (sudo pide contraseña, no hay TTY); deja el comando:
cargo xtask flash-usb-live /dev/sdX --yes --only kernel
```

VFIO: `SOSO_WIFI_BDF` (default `80:14.3`), log `target/wifi-vfio-serial.log`.
QEMU nic: `SOSO_QEMU_NIC=vfio:<BDF>` + `SOSO_LXDDE_MODE=iwlwifi`.

## Gotchas

- **Ethernet viva en live con iwlwifi activo.** El `e1000e` nativo se inicializa
  salvo que el *port* lxdde `e1000e` lleve ese chip (`if !modes.e1000e`).
  Gatearlo también por `modes.iwlwifi` deja la ethernet muerta: el live siempre
  lleva iwlwifi compilado.
- **No hay fallback slirp en WiFi ni rtl8169 de placa.** Sin lease DHCP no hay
  10.0.2.15.
- **PNVM solo gen3.** AX200 no lo pide; AX211 sí (`iwl_fw_parse_pnvm`).
- **Anillo RX: dos formatos, no uno (2026-09-15).** gen3 usa
  `iwl_rx_transfer_desc` (16 B, `rbid` fuera de la dirección) y
  `iwl_rx_completion_desc` (32 B, `rbid` en el offset 4); gen2 usa `__le64
  (addr|vid)` y un `__le32`. El VID va de 1 a N y designa el buffer `VID-1`; el
  0 es «ranura vacía». Con el formato de gen2 en gen3 el síntoma es `timeout
  ALIVE` con recepciones vacías, no un fallo de firmware.
- **Descriptor de MPDU: 48 B en AX200, 56 en AX211 (`IWL_RX_DESC_SIZE_V3`).**
  Nunca los 4 de `iwl_rx_mpdu_res_start`: eso desplaza la trama 802.11 52 B.
- **`DATA_PATH_GROUP` id 1 es `UPDATE_MU_GROUPS_CMD`**, no una notificación de
  Ethernet. Los datos llegan por `REPLY_RX_MPDU_CMD` y hay que convertirlos
  (`iwl_mvm_rx_to_eth`); entregar aquel cuerpo a smoltcp era inventar paquetes.
- **TX de datos: ToDS y LLC/SNAP.** `frame[1]=0x02` es FromDS (lo que manda un
  AP); una estación pone `0x01`. `addr3` es el **destino Ethernet**, no el
  BSSID. La carga va tras LLC/SNAP, no es la trama 802.3 entera.
  `IWL_TX_FLAGS_ENCRYPT_DIS` sólo mientras no haya claves: con la API nueva de
  TX el firmware pone cabecera CCMP, PN y el bit Protected.
- **Asociada ≠ autorizada.** `iwl_ax211_connected()` es la asociación;
  `iwl_ax211_authorized()` es lo que puede usar IP. `net::wifi_link_up()` y
  `WIFI_FLAG_AUTHORIZED` miran la segunda: con la primera, DHCP arrancaba antes
  de que hubiera claves.
- **EAPOL tiene cola propia** (`iwl_ax211_rx_eapol`). Compartirla con los datos
  deja que smoltcp se lleve M1/M3 y el 4-way se cuelga sin decir nada.
- **La GTK necesita `STA_KEY_MULTICAST`, su Key ID del KDE y ranura propia**
  (`iwl_mvm_install_gtk`). Sin el bit, el firmware la registra como otra clave
  de pares y el tráfico de difusión no se descifra. CCMP cifra con la **TK**
  (`ptk[32..48]`), nunca con la KCK.
- **AX200 gen2 timeout ALIVE (INT=0):** restockear el anillo RX con
  `rx_write = IWL_GEN2_RX_N - 1` antes de `UREG_CPU_INIT_RUN` (como gen3). Con
  WIDX=0 el firmware no recibe RBD y no manda `UCODE_ALIVE_NTFY`.
- **Nunca dejar `memcmp`/`memcpy`/… como dummy.** `lx-build` stubbea lo
  undefined con `lx_emul_trace_and_stop`. Ese símbolo se enlaza al kernel y
  pisa el de `compiler_builtins`: el live se clava en `boot: live-disk` con
  `lx: lxdde: stub trace: memcmp` al comparar el GPT. Implementar en
  `lxdde/shim/src/shims.c` y listar en `provided_symbols()`.
- **Nunca dejar `memcmp`/`memcpy`/… como dummy.** `lx-build` stubbea lo
  undefined con `lx_emul_trace_and_stop`. Ese símbolo se enlaza al kernel y
  pisa el de `compiler_builtins`: el live se clava en `boot: live-disk` con
  `lx: lxdde: stub trace: memcmp` al comparar el GPT. Implementar en
  `lxdde/shim/src/shims.c` y listar en `provided_symbols()`.
- **Nunca dejar `memcmp`/`memcpy`/… como dummy.** `lx-build` stubbea lo
  undefined con `lx_emul_trace_and_stop`. Ese símbolo se enlaza al kernel y
  pisa el de `compiler_builtins`: el live se clava en `boot: live-disk` con
  `lx: lxdde: stub trace: memcmp` al comparar el GPT. Implementar en
  `lxdde/shim/src/shims.c` y listar en `provided_symbols()`.
- **QEMU no emula ninguna tarjeta WiFi.** No hay modelo 802.11 upstream (QEMU
  8.2 lista 0). `mac80211_hwsim` y `virt_wifi` simulan por encima de mac80211,
  que soso no usa: no sirven para el driver. El único camino con silicio es
  VFIO. Lo que **sí** aporta hwsim es un AP real (hostapd) para capturar un
  4-way y contrastar el supplicant con una implementación ajena:
  `scripts/l6-wifi-capture-4way.sh`.
- **Un modelo de firmware no descubre formatos.** `ring_soak` caza que las dos
  mitades del driver se contradigan bajo carga (buffer publicado dos veces,
  reciclado antes de tiempo, TFD reutilizado sin confirmar), pero sale de la
  misma lectura de la especificación que el driver: si el formato está mal, los
  dos se equivocan igual. Por eso el arreglo de descriptores necesita placa.
- **La secuencia TX sólo lleva 8 bits de índice.** Una respuesta repetida de un
  TFD ya reutilizado es indistinguible de la legítima; `iwl_trans_tx_reclaim`
  sólo puede ignorar las que apuntan por detrás de la cabeza, igual que
  `iwl_txq_reclaim` de Linux. No pidas más al driver.
- **Hostcheck antes de gastar un ciclo VFIO.** Compila `iwl_fw.c` en host contra
  los `.ucode` del rootfs; afirma lmac/umac de `SEC_RT`, CMD_VERSIONS/PHY_SKU,
  doorbell `qid<<16` (`0x00000001`, cola HCMD=0), secuencia `QUEUE_TO_SEQ|INDEX_TO_SEQ`, y
  tamaño fijo SCAN_REQ_UMAC v14–17 (`sizeof` 2112 B, `channel_config[67]`;
  `count` no desplaza periodic/probe). Flags V2: PASS_ALL=BIT(1),
  ITER_COMPLETE=BIT(2). MAC CSR en `mac_addr_from_csr=0x380`, no 0x000/008.
- **Doorbell HBUS_TARG_WRPTR (2026-09-09).** `write_ptr | (qid << 16)`, no
  `qid << 8`. Cabecera wide: `QUEUE_TO_SEQ(qid) | INDEX_TO_SEQ(slot)`; versión
  del cmd desde TLV CMD_VERSIONS.
- **Cola HCMD (2026-09-09).** `IWL_MVM_DQA_CMD_QUEUE=0` (Linux 6.6). El 9 es
  `IWL_MVM_DQA_AP_PROBE_RESP_QUEUE`; doorbell erróneo deja al FW sordo tras ALIVE.
- **Init MVM tras ALIVE (2026-09-09).** `iwl_mvm_run_init()`: INIT_EXTENDED_CFG
  (`init_flags=1<<IWL_INIT_NVM` = 2) → NVM_ACCESS_COMPLETE → PHY_CFG sólo gen3 →
  esperar `INIT_COMPLETE_NOTIF` antes de scan. Sin INIT_COMPLETE no marcar radio lista.
- **Scan UMAC v14–17.** Estructura completa v17 (2112 B). Versiones
  explícitas 6 y 14–17; el resto se rechaza. Flags V2, no los BIT(2)/BIT(5)
  viejos. Canales desde perfil NVM/MCC (activo/pasivo); fallback 2.4+5.
  Fin de scan por UID: vacío+COMPLETE es resultado vacío; BSS sin fin no
  acredita scan. HCMD: match grupo+id+seq, no notif (`SEQ_RX_FRAME`), no
  pisar pending, timeout envenena el slot.
- **RX post-ALIVE (2026-09-09).** AX200 (`!gen3`): saltar `IWL_RX_DESC_SIZE_V1`
  (48 B) en MPDUs; AX211 usa `iwl_rx_mpdu_desc`. `SCAN_COMPLETE_UMAC` acepta grupo
  legacy además de LONG. Log RX: `(grp,id,seq,len)`.
- Credenciales: ESP primero (`wificonf`), luego `/etc/wifi.conf`. El hueco ESP
  es 4 KiB pre-creado y contiguo (mismo patrón `espfat` que SOSOLOG).

Detalle de red/SSH/hwscan: skill **`soso-architecture`**. Empaquetado live:
**`soso-live`**.

## Matriz hardware (A8)

Entradas seed: `ax211-wifi` (`8086:7f70`), `ax200-wifi` (`8086:2723`) en
[`docs/hw-matrix.json`](../../docs/hw-matrix.json).

Tras SOSOLOG de placa, el agente **sí** actualiza la matriz (`parse-logs`
`--id ax211-wifi`). Monta la ESP con `udisksctl` (skill **soso-live**), no
`cargo xtask sosolog`. No uses `--boot-ok` ni marques `alive: ok` sin
`UCODE_ALIVE_NTFY` en el log. `timeout ALIVE` / `alive=false` = fail.

```bash
./scripts/l6-iwl-fw-hostcheck.sh          # host, ~1 s
sudo ./scripts/l6-wifi-vfio-test.sh       # VFIO → ALIVE real
cargo xtask hw-matrix parse-logs --id ax211-wifi --sosolog SOSOLOG.TXT --sosodrv SOSODRV.TXT
cargo xtask hw-matrix show
```

Guía: [`docs/HW-MATRIX.md`](../../docs/HW-MATRIX.md). Estado: [`docs/ESTADO.md`](../../docs/ESTADO.md).
