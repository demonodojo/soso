# ROG: red WiFi — detección y mitigación (24 oct 2026)

## Contexto

Intel **AX200** (`8086:2723`). Síntomas en placa: DNS mudo sin línea del driver
(11 oct USB), `SW_ERR` tras inferencia GPU, desaparición de SSH sin log guardado.

## Cambios en código (fase 1 + 2)

| Área | Qué |
|---|---|
| `iwl_trans.c` | Punteros de error del ALIVE; volcado LMAC/UMAC en `SW_ERR`; `CT_KILL`/`RF_KILL`; vigilante **RX parado** (causa 6); anillo **256** RBD |
| `kernel/…/wifi.rs` | Huecos de sondeo >500 ms; `sondeo_max` en pérdidas |
| `kernel/…/wifi_wpa.rs` | Estado de reconexión cada 60 s |
| `kernel/…/timer.rs` | Bombeo `net::poll` + `wifi::poll` en `sleep_ms` (BSP) |
| `kernel/…/task/mod.rs` | Yield forzado si `mantener` >100 ms |
| Hostcheck | `fw_errlog_test`, `ring_soak` RX parado |

## Prueba en placa (pendiente operador)

1. Flashear kernel: `cargo xtask flash-usb-live /dev/sdX --yes --only kernel`
2. Host: `./scripts/l6-rog-red-watch.sh <IP> target/rog-red-YYYY-MM-DD`
3. Escenarios del plan (reposo, `soso-update aplicar`, `ask` CPU/GPU, 1 h, AP off)
4. Recoger: `./scripts/l6-rog-red-hw-run.sh` o ESP `SOSOLOG.TXT`

## Criterio de aceptación

- Cada caída con línea de causa (`error fw`, TX/RX atascada, sondeo parado, DEAUTH/beacons)
- Tras mitigaciones: sin `sondeo parado` >500 ms durante `ask`/GPU; reconexión automática

## Host (esta sesión)

- `./scripts/l6-iwl-fw-hostcheck.sh` con `TMPDIR=target/iwl-hostcheck` (evita cuota `/tmp`).
- Bancos nuevos: `fw_errlog` (ALIVE + volcado LMAC sintético), `ring_soak` RX parado / SW_ERR.

## Matriz (`ax200-wifi`)

Tras placa: `cargo xtask hw-matrix parse-logs --id ax200-wifi --sosolog target/rog-red-*/SOSOLOG.txt`.
Marcadores útiles en el log: `iwl_trans: error fw`, `RX parado`, `wifi: sondeo parado`, `wifi: reconexión`.
