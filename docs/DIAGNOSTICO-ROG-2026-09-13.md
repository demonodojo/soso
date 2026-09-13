# ROG: teclado USB 18c6 — diagnóstico (13 sep 2026)

## Evidencia y alcance

Lecturas ESP `/dev/sda1` (`KERNEL`) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-13-run2/` (arranque 06:38, kernel local con
fixes parciales) y comparación con `target/usb-diagnostic-2026-09-12-run5/`
(6 arranques OK con `29056871b-dirty`).

| Campo | Valor |
|---|---|
| Kernel USB (run2) | **0.2.2 (`3b88125cb-dirty`)** |
| Hardware | `10de:249c` (GA107) + `8086:2723` (AX200) + `10ec:8168` (rtl8169) |
| Userspace | **`sosh —`** + marca OTA `pid=2 write=182ms` + prompt `$` |
| Síntoma | **Teclado muerto: `kbd sc=0 enc=0 ent=0` @ 79554 ms** |
| SOSOWIFI | vacío |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `hub.c:5380–5514` (PORT_INIT_TRIES), `usb.h:1906` (USB_CTRL_SET_TIMEOUT 5000 ms), `hid-core.c:999–1002` |
| `lxdde/linux/drivers/hid/hid-ids.h` | `0b05:1866` N-Key, sin entrada `0b05:18c6` (teclado interno ROG) |

Hostchecks (no validan teclado): `l6-iwl-fw-hostcheck.sh` OK,
`l6-g3-gsp-hostcheck.sh` OK.

---

## Tabla de etapas (run2 = flush #17 @ 79554 ms)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA `pid=2 write=182ms` | **OK** |
| Teclado | flush #17: `kbd sc=0 ultimo=0x00 enc=0 ent=0` | **FAIL** |
| USB HID | `0b05:1866` slot 2 boot; `0b05:18c6` slot 3 **omitido SET_CONFIGURATION** | **FAIL** |
| GSP / RPC | `GSP_INIT_DONE`, `pool VRAM=sí` | **OK** |
| WiFi scan | `SCAN_REQ v15` → `scan fin count=22` | **OK** |
| Ethernet | `phystatus 0x84` → DOWN | **FAIL** (sin cable) |
| DHCP | `net: dhcp…` sin lease | **pendiente** |

**Referencia run5 (29056871b, teclado OK):** flush #39 `kbd sc=139 enc=69 ent=69`;
`SET_PROTOCOL(Boot) succeeded` ×2; `keyboard ready on slot=3` (`0b05:18c6`).

---

## Hallazgos

### USB-1. El teclado físico es `0b05:18c6`, no `0b05:1866` ni PS/2 (confirmado)

**Evidencia run5 SOSODRV:** slot 2 `0b05:1866` (248 B rep, Aura/hotkeys, sin
colección `Usage 0x06`); slot 3 `0b05:18c6` (231 B rep, colección teclado
Report ID 1, array 30 teclas + bitmap). i8042 responde a probe pero no entrega
teclas en ningún arranque reciente (`sc=0` o un solo Shift fantasma).

**Linux:** enumera todos los HID boot (`03:01:01`) con `usb_set_configuration`
(`generic.c:228–262`); no salta dispositivos por VID.

### USB-2. Regresión A: SET_CONFIGURATION timeout sin reintento (confirmado)

**Síntoma (arranque 06:00, HEAD sin fixes):** `SET_CONFIGURATION timeout (5000 ms)`
en slot 3 → `disable_slot` sin reintento.

**soso:** `driver.rs` fallaba una vez y liberaba el slot.

**Linux:** `hub.c:5380–5514` — hasta **4** intentos con reset de puerto
(`PORT_INIT_TRIES`).

**Fix:** timeout EP0 5 s + bucle de reintento con `reset_port` (este ciclo).

### USB-3. Regresión B: salto explícito del 18c6 + sin SET_PROTOCOL Boot (confirmado)

**Síntoma (run2 06:38):** `omitiendo SET_CONFIGURATION slot=3 VID=0x0b05`;
`keyboard ready on slot=2` solo (1866); `kbd sc=0`.

**soso:** `device.rs needs_full_config` devolvía `false` para `0b05:18c6`;
`setup_keyboard` dejó de mandar `SET_PROTOCOL(Boot)`.

**Linux:** `hid-core.c` usa report protocol por defecto; en placa el camino
validado es boot (run5: `SET_PROTOCOL(Boot)` + layout 8 bytes).

**Fix:** quitar salto 18c6; restaurar `SET_PROTOCOL(Boot)` + `KeyboardReportLayout::boot()`
si `boot == true` (este ciclo).

### Descartados (ciclo anterior, no causa de run2)

- IRQ1 / `try_lock` en `kbd.rs`: run2 llegó a 17 flushes; mass storage OK.
- Parser N-Key 1866: el 1866 no es el teclado principal en esta placa.
- iwlwifi sucio: sin bucles nuevos en `lxdde::poll`; WiFi scan OK en run2.

---

## Orden de corrección (implementado)

1. **`device.rs` / `driver.rs`:** quitar salto `0b05:18c6`; `EP0_TRANSFER_TIMEOUT_US`
   = 5 s; reintento SET_CONFIGURATION con reset de puerto (×4, Linux `hub.c`).
2. **`driver.rs setup_keyboard`:** `SET_PROTOCOL(Boot)` + layout boot si iface boot;
   log `keyboard ready (VID:PID)`; `kbd: usb hid activo (N)`.
3. **`hid.rs` tests:** descriptor real 231 B del 18c6; 1866 System Control ≠ teclado.
4. **Este informe + `hw-matrix.json`** (logs run2; teclado `fail` hasta validar en placa).

---

## Validación en placa (usuario)

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

Tras arrancar y teclear en `$`: SOSOLOG debe mostrar `keyboard ready on slot=3
(0x0b05:0x18c6)`, `SET_PROTOCOL(Boot) succeeded`, `kbd: usb hid activo (2)` y
cabecera con `sc>0 enc>0 ent>0`.

---

## Qué no se ha hecho

- No se ha reflasheado ni validado en placa en este ciclo.
- WiFi assoc / DHCP / ethernet sin cable: fuera de alcance.
- Report protocol del 18c6 (62 B): tests host listos; producción usa boot.

---

# ROG run3 — WiFi assoc BINDING 0x2b (13 sep 2026, ~21 min)

## Evidencia

Lectura ESP `/dev/sda1` (`KERNEL`), kernel **0.2.2 (`3b88125cb-dirty`)**, flush **#37 @
1282698 ms**. Copias en `target/usb-diagnostic-2026-09-13-run3/` (no al git).

| Campo | Valor |
|---|---|
| Hardware | `10de:249c` GA107 + `8086:2723` AX200 + `10ec:8168` rtl8169 |
| Userspace | **`sosh —`** + OTA `pid=2 write=165ms` |
| Teclado | **`kbd sc=149 enc=74 ent=74`**; `keyboard ready on slot=3 (0x0b05:0x18c6)` + `SET_PROTOCOL(Boot) succeeded` |
| GPU | `GSP_INIT_DONE`, `pool VRAM=sí`, CE readback verificado |
| WiFi scan | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, scan `count=22` |
| WiFi assoc | `PHY_CONTEXT ch3 band=1 action=2 ok` → **`timeout cmd grp=1 id=0x2b slot=17`** → `BINDING assoc falló` |
| Ethernet | `phystatus 0x84` DOWN (sin cable; faltaba `rtl_hw_start_8168h_1`) |

## Tabla de etapas (run3)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA ok | **OK** |
| Teclado USB 18c6 | `kbd sc=149 enc=74 ent=74`, SET_PROTOCOL Boot | **OK** (fix run2 validado) |
| GSP / RPC | `GSP_INIT_DONE`, pool VRAM, CE readback | **OK** |
| WiFi scan | `scan fin count=22` | **OK** |
| WiFi assoc | `timeout … id=0x2b` (BINDING MODIFY extra) | **FAIL** |
| Ethernet | `phystatus 0x84` DOWN | **FAIL** (sin cable + MAC start 8168H) |

## Causa raíz (vs Linux 6.6.32)

- **`iwl_mvm_assoc_prepare`** reenviaba `BINDING_CONTEXT MODIFY (0x2b)` tras cambiar
  canal PHY; Linux solo toca BINDING en add/remove de vif, no al cambiar canal
  (`phy-ctxt.c:285–322`, `binding.c:90–129`).
- Faltaba **`TIME_EVENT_CMD 0x29`** (`TE_BSS_STA_AGGRESSIVE_ASSOC`) antes de TX AUTH
  (`mac80211.c:2453`, `time-event.c:599`).
- **`ADD_STA_KEY`** usaba struct/grupo inventados; Linux: LEGACY `0x17`, layout
  `sta.h:361–399`.
- **rtl8168H:** faltaba `rtl_hw_start_8168h_1`; `phystatus 0x80` no es TBI en VER_46.

## Fixes aplicados (código, pendiente placa)

1. Omitir BINDING en assoc si PHY0↔MAC0 ya existe; CDB: REMOVE → PHY → ADD solo
   si cambia banda.
2. `TIME_EVENT 0x29` tras ADD_STA y antes de AUTH TX.
3. `ADD_STA_KEY` ABI Linux v2 (LEGACY, `STA_KEY_FLG_CCM`).
4. `rtl_hw_start_8168h_1` + no etiquetar TBI en 8168H.
5. Hostcheck: `./scripts/l6-iwl-fw-hostcheck.sh` verde; `assoc_abi_test` sin exigir
   `0x2b`.

## Validación en placa (usuario)

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

Esperado tras `wifi connect`: sin `timeout … id=0x2b`; siguiente HCMD = MAC/ADD_STA;
no AUTH timeout inmediato; ethernet `rtl_hw_start_8168h_1 ok` y enlace UP con cable.
