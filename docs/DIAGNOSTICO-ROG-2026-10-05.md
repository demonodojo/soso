# ROG RTX 3080 Laptop: diagnóstico (5 oct 2026)

## Evidencia y alcance

| Campo | Valor |
|---|---|
| ESP | `/dev/sda1` (`KERNEL`, vfat), desmontada tras copia |
| Kernel (banner) | **soso 0.3.7 (cd5f6a36a-dirty)** |
| Flush / uptime | **#30**, **185861 ms** (~3 min) |
| PCI relevante | `10de:249c` GA104 iGPU, `8086:2723` AX200, `10ec:8168` rtl8169 |
| Copias | `target/usb-diagnostic-2026-10-05/` (`SOSOLOG.body.txt`, `SOSODRV.txt`) |
| Árbol Linux | `lxdde/linux/` (6.6.32, iwlwifi + r8169) |
| Hostcheck | `l6-iwl-fw-hostcheck.sh` OK; `l6-g3-gsp-hostcheck.sh` OK (no sustituyen placa) |

`SOSOWIFI.TXT` existe en ESP pero **vacío** (4096 B de relleno). `SOSOKRN.MET` / `SOSOUPD.TXT` vacíos en esta sesión.

## Tabla de etapas

| Etapa | Cita SOSOLOG | Resultado |
|---|---|---|
| Shim UEFI | `soso-shim: UEFI alcanzado` | OK |
| Kernel `boot:` | hasta `boot: task` | OK |
| Userspace | `sosh — escribe 'help' para la ayuda` | OK |
| GPU GSP | `GSP_INIT_DONE recibido`, `pool VRAM=sí` | OK |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`, `alive=true` | OK |
| WiFi scan | `scan fin count=21 end=1` | OK |
| WiFi assoc | `'soso-open' no está entre los 21 BSS del scan` | **Fallo** |
| Ethernet | `phystatus … → enlace DOWN`, `net: dhcp…` sin `net: dhcp x/x` | **Sin IP** |
| Consola | `ip`/`dns` errno 107; `dhcp: no existe` | Fallo usuario esperado |

## Hallazgos

### 1. Autoconnect a SSID demo con SOSOWIFI vacío (confirmado)

- **Síntoma:** scan devuelve 21 BSS pero no hay asociación ni `wifi: conectado (autoconnect)`.
- **Log:** `iwl_mvm: 'soso-open' no está entre los 21 BSS del scan`.
- **soso:** `kernel/src/net/wifi_wpa.rs:26-37` — si `SOSOWIFI.TXT` no parsea `ssid=`, cae a `/etc/wifi.conf`; `rootfs/etc/wifi.conf` fija `ssid=soso-open`.
- **soso:** `kernel/src/drivers/wificonf.rs:3-4` documenta ese fallback explícitamente.
- **Linux:** `lxdde/ports/iwlwifi/iwl_mvm.c:753-767` — mismo criterio: sin BSS coincidente, `-1` y mensaje (port alineado con `drivers/net/wireless/intel/iwlwifi/mvm/mac80211.c` scan+connect).
- **Causal:** **confirmado** — no es fallo de scan TX/RX; el SSID configurado no está en el aire (o la red real no es `soso-open`).

### 2. DHCP cableado sin carrier (confirmado, mejora UX)

- **Síntoma:** `net: dhcp…` al arranque pero nunca `net: dhcp a/b`; `ip` falla en sosh.
- **Log:** `rtl8169: phystatus 0x84 bmsr 0x7989 → enlace DOWN 10M half` (sin cable / sin link partner).
- **soso:** `kernel/src/net/mod.rs:289-290` — `dhcp_now = true` para cualquier backend cableado aunque el PHY esté DOWN; `on_wired_link_up()` (`337-355`) sólo dispara si `poll_link()` ve transición UP.
- **Linux:** `lxdde/linux/drivers/net/ethernet/realtek/r8169_main.c:4637-4653` — `r8169_phylink_handler` actúa sobre `netif_carrier_ok`; no arranca tráfico IP útil sin carrier.
- **Causal:** **confirmado** — entorno sin ethernet; mejorable diferir DHCP hasta carrier (menos ruido y mensajes más claros).

### 3. Comando `dhcp` ausente en sosh (confirmado)

- **Síntoma:** `sosh: dhcp: no existe`.
- **soso:** `user/sosh/src/main.rs` — builtins `help` lista `wifi`/`ask` pero no `dhcp`.
- **Causal:** **confirmado** — gap de UX, no del stack smoltcp.

### 4. xHCI timeout en hub USB (recuperado, hipótesis baja)

- **Log:** `transfer event timeout after 5000 ms slot=1` → `EP recovered` → hub `048d:1234` enumerado.
- **Impacto en sesión:** teclado HID ASUS enumeró; no panic. Seguimiento sólo si el teclado falla en uso.

## Orden de corrección

1. **`read_wifi_config_text` / live ESP** — Si `wificonf::read_text()` devuelve slot ESP pero `parse_wifi_conf` falla (fichero vacío), **no** usar `/etc/wifi.conf` demo; log `wifi: SOSOWIFI vacío — escribe ssid=/psk= en la ESP o wifi connect` y omitir autoconnect. Archivos: `kernel/src/net/wifi_wpa.rs`, `kernel/src/drivers/wificonf.rs` (comentario). Validación: QEMU/live sin SOSOWIFI → no línea `soso-open`; con SOSOWIFI válido → autoconnect como hoy.

2. **DHCP rtl8169 sin carrier** — En `attach_now` / `attach_stack`, `dhcp_now` para `BackendKind::Wired` sólo si `rtl8169::link_up()` (exportar getter) o tras `on_wired_link_up`; log `net: sin enlace cableado — DHCP diferido`. Linux ref: carrier en `r8169_phylink_handler`. Validación: boot sin cable → sin reintentos DHCP ruidosos; enchufar cable → `net: enlace ethernet UP — solicitando DHCP…` y lease.

3. **`sosh dhcp`** — Builtin que imprima estado (`configured`, dirección, DNS) leyendo la misma API que `ip` o syscall existente; documentar en `MANUAL-USUARIO.md` si aplica. Validación: sosh en placa tras WiFi OK.

4. **Operativa inmediata (sin código)** — Rellenar `SOSOWIFI.TXT` en la ESP con la red real (`ssid=`/`psk=`) o `wifi connect …` en consola; comprobar que el SSID aparece en el scan de 21 BSS.

5. **Registro** — Matriz `ga107-igpu` / `ax200-wifi` y este informe (hecho en esta ejecución).

## Qué no se ha hecho

No se modificó código de drivers, no se reflasheó el USB, no se incrementó `arranques_consecutivos_ok`.
