---
name: soso-wifi
description: >-
  soso WiFi Intel (iwlwifi/lxdde) — AX211/AX200, firmware TLV, WPA2 EAPOL,
  VFIO and hostcheck. Use when modifying lxdde/ports/iwlwifi, net/wifi_wpa.rs,
  wificonf, SOSOWIFI.TXT, wifi syscalls, scripts l6-wifi-* / l6-iwl-*, or
  diagnosing scan/assoc/ALIVE on live USB or QEMU passthrough.
---

# soso — WiFi Intel (iwlwifi)

Driver **first-party** en `lxdde/ports/iwlwifi/` (no es un port de mac80211
completo). Transporte PCIe + firmware TLV + MVM scan/assoc/TX mínimos.
Supplicant WPA2 en kernel Rust (`kernel/src/net/wifi_wpa.rs`).

## Chips

| PCI ID | Chip | Context-info | Firmware | PNVM |
|--------|------|--------------|----------|------|
| `8086:7f70`, `51f0`, `54f0` | AX211 | gen3 | `iwlwifi-so-a0-gf-a0-{89,77}.ucode` | `iwlwifi-so-a0-gf-a0.pnvm` |
| `8086:2723` | AX200 | gen2 | `iwlwifi-cc-a0-{77,74,73,72,66}.ucode` | no |

`iwl->gen3 = device_id != 0x2723`. El live perfil `live-usb` ya incluye
`SOSO_LXDDE_MODE=nouveau,iwlwifi`.

Firmware en `rootfs/lib/firmware/`. `lx_request_firmware` prueba alternativas
en silencio; el fallo lo canta el llamante (`iwl_ax211: firmware no encontrado`).

## Archivos

| Path | Rol |
|------|-----|
| `iwl_ax211.c` | probe PCI, carga ucode/pnvm, arranque gen2/gen3 |
| `iwl_fw.c` | parser TLV (`SEC_RT` lmac/umac, paging, pnvm) |
| `iwl_trans.c` | context-info, colas MTR/MCR/RX, espera ALIVE |
| `iwl_mvm.c` | scan UMAC, assoc, TX |
| `iwlwifi_lx.c` | exports C → Rust (`lx_iwlwifi_*`) |
| `lxdde/shim/src/{iwlwifi,skbuff,netdev,cfg80211,mac80211}_lx.c` | shims mínimos |
| `kernel/src/net/wifi_wpa.rs` | PBKDF2-PSK + 4-way EAPOL + CCMP |
| `kernel/src/drivers/wificonf.rs` | lee `SOSOWIFI.TXT` (ESP, 8.3) |
| `tools/iwl-hostcheck/main.c` | parser TLV contra blobs reales |

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
sudo ./scripts/l6-wifi-vfio-test.sh       # VFIO AX211 → QEMU; GO = ALIVE real
# Live (nouveau+iwlwifi ya van): editar SOSOWIFI.TXT en ESP p1.
# Agente: monta p1 con udisksctl (skill soso-live, sin sudo/TTY); no uses cargo xtask sosolog.
# El agente no graba el USB (sudo pide contraseña, no hay TTY); deja el comando:
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only kernel
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
- **AX200 gen2 timeout ALIVE (INT=0):** restockear el anillo RX con
  `rx_write = IWL_GEN2_RX_N - 1` antes de `UREG_CPU_INIT_RUN` (como gen3). Con
  WIDX=0 el firmware no recibe RBD y no manda `UCODE_ALIVE_NTFY`.
- **Hostcheck antes de gastar un ciclo VFIO.** Compila `iwl_fw.c` en host contra
  los `.ucode` del rootfs; afirma lmac/umac de `SEC_RT`.
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
