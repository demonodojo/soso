# GB205 + AX211 — MAC_CONTEXT timeout (17 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-17/`](../target/usb-diagnostic-2026-09-17/).
Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | Valor (flush **#37**, último arranque en el log) |
|---|---|
| Kernel en placa (SOSOLOG) | **0.2.2 (`a8f6be44f-dirty`)** |
| Flush / uptime | **#37 @ 161 017 ms** (~2,7 min); ~68 KiB / 256 KiB |
| Hardware | `10de:2f18` GB205 + `8086:7f70` AX211 + `10ec:8125` RTL8125 **sin driver** |
| Userspace | **`sosh —`** pid=2 |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; CE selftest OK |
| WiFi | ALIVE + PNVM doorbell **0xFE** + NVM OK; **`timeout cmd grp=1 id=0x28`** (MAC_CONTEXT scan) → `recover_fail` → `wifi scan: error de E/S` |
| Ethernet | `RED SIN DRIVER 83:00.0 10ec:8125` |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — MLD MAC [`mvm/mld-mac.c`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mld-mac.c), [`fw/api/mac-cfg.h`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/mac-cfg.h) |

Firmware en rootfs:

| Chip | Imagen | MAC scan |
|---|---|---|
| AX211 | `iwlwifi-so-a0-gf-a0-89.ucode` | **MAC_CONFIG** grp3 id=8 + **LINK_CONFIG** id=9 (sin `0x28`) |
| AX200 (ROG) | `iwlwifi-cc-a0-77.ucode` | **MAC_CONTEXT** grp1 id=`0x28` ver=5 (legacy OK mismo kernel) |

Hostchecks (tras fix MLD en árbol):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK** (incl. `mld_mac_route`)
- `./scripts/l6-g3-gsp-hostcheck.sh` — no re-ejecutado en esta pasada; sin cambios GSP

**Placa:** el log anterior refleja el kernel **antes** del reflash con MAC_CONFIG MLD.
Tras flashear: `cargo xtask flash-usb-live /dev/sda --yes --only kernel`.

Matriz: [`hw-matrix.json`](hw-matrix.json) — `gb205-dgpu`, `ax211-wifi` (sin `record-boot`; racha WiFi reseteada).

---

## Tabla de etapas (flush #37, kernel en log)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (a8f6be44f-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#37** @ ~161 s | OK |
| USB live | GPT backend=Usb | **OK** |
| GPU GSP | `RPC fn=0x1001 (GSP_INIT_DONE) res=0x0` | **OK** |
| GPU VRAM / CE | `pool VRAM=sí`; `CE selftest OK` | **OK** |
| GPU compute | sm_120 kernels cargados; sin `ask` en sesión | **pendiente** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; PNVM publish + SKU **0xFE** | **OK** |
| WiFi NVM | `init NVM listo (phy_sku=0x00330018 n_scan=67)` | **OK** |
| WiFi MAC scan | `timeout cmd grp=1 id=0x28` ×4 recuperaciones | **FAIL** |
| WiFi scan user | `phase=recover_fail`; `wifi scan: error de E/S` | **FAIL** |
| RTL8125 | `10ec:8125 sin-driver` | etapa ausente |

Secuencia WiFi (representativa):

```
RLC_CONFIG ok → send MAC_CONTEXT 0x28 (scan) → timeout slot=9
→ «MAC_CONTEXT scan timeout — sigue up» → cola POISON
→ SCAN_CFG v5 falló → recover ×3 → recover_fail
```

---

## Causa raíz (vs Linux 6.6)

El ucode **so-a0-gf-a0-89** declara API MLD (`IWL_UCODE_TLV_CAPA_MLD_API_SUPPORT`) y
**no** expone `(grp=1, cmd=0x28)` en TLV CMD_VERSIONS. Linux enruta por
`iwl_mvm_has_mld_api` a **MAC_CONFIG** (`MAC_CONF_GROUP`, id=8) y **LINK_CONFIG** (id=9).

soso (kernel del log) enviaba siempre **MAC_CONTEXT 0x28** en `iwl_mvm_mac_ctxt_add_scan`
y, al timeout, **seguía el up** (`return 0`), envenenando la cola HCMD y haciendo fallar
**SCAN_CFG** en cascada.

---

## Corrección en árbol (implementada)

| Ítem | Cambio |
|---|---|
| Ruta MAC | `iwl_mvm_uses_mld_mac()` en [`iwl_mvm_route.c`](../lxdde/ports/iwlwifi/iwl_mvm_route.c); up scan usa MAC_CONFIG + LINK ADD/MODIFY |
| Abort up | Fallo MAC/LINK → `-1`; no SCAN_CFG sobre cola bloqueada |
| BINDING 0x2b | Omitido si el ucode no declara `BINDING_CONTEXT_CMD` (89) |
| Assoc | [`iwl_mvm_assoc.c`](../lxdde/ports/iwlwifi/iwl_mvm_assoc.c): MODIFY MAC_CONFIG + beacon timing en LINK |
| Host | [`mld_mac_route_test.c`](../tools/iwl-hostcheck/mld_mac_route_test.c) + script hostcheck |

---

## Fuera de alcance inmediato

- Driver **Realtek RTL8125** (`10ec:8125`).
- Validación silicio post-reflash (scan count>0, connect, DHCP).
- `ask` / compute GPU en esta sesión.
