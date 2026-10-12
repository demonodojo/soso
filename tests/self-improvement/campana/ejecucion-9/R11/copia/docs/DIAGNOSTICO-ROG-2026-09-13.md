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

---

# ROG run4 — PHY_CONTEXT LMAC CDB (13 sep 2026, flush #32)

## Evidencia

Lectura ESP `/dev/sda1` (`KERNEL`), kernel **0.2.2 (`3b88125cb-dirty`)**, flush **#32 @
151769 ms**. Copias en `target/usb-diagnostic-2026-09-13/` (no al git).

| Campo | Valor |
|---|---|
| Hardware | `10de:249c` GA107 + `8086:2723` AX200 + `10ec:8168` rtl8169 |
| Userspace | **`sosh —`** + OTA `pid=2 write=181ms` |
| Teclado | **`kbd sc=113 enc=56 ent=56`**; `keyboard ready on slot=3 (0x0b05:0x18c6)` |
| GSP | `GSP_INIT_DONE`, `pool VRAM=sí`, `compute listo cls=0xc7c0` |
| WiFi scan | `UCODE_ALIVE_NTFY`, scan `count=24` / 21 / 19 |
| WiFi assoc | BINDING REMOVE `action=3 ok` → **`timeout cmd grp=1 id=0x08`** → `PHY_CONTEXT REMOVE ch40 falló` |
| Ethernet | `rtl_hw_start_8168h_1 ok`; `phystatus 0x84` DOWN (sin cable) |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/binding.c:168–173` (`iwl_mvm_get_lmac_id`), `mvm/phy-ctxt.c:300–314` |

Firmware `iwlwifi-cc-a0-77.ucode`: capa **39** (BINDING_CDB) = sí, capa **40** (CDB) = no.

Hostchecks: `./scripts/l6-iwl-fw-hostcheck.sh` OK (incl. `cdb_lmac_test`).

## Tabla de etapas (run4)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA ok | **OK** |
| Teclado USB 18c6 | `kbd sc=113`, SET_PROTOCOL Boot | **OK** |
| GSP / RPC | `GSP_INIT_DONE`, pool VRAM, compute | **OK** |
| WiFi scan | `scan fin count=24` | **OK** |
| WiFi assoc | `timeout … id=0x08` tras BINDING REMOVE | **FAIL** |
| Ethernet | `phystatus 0x84` DOWN | **FAIL** (sin cable) |

## Hallazgo confirmado

**soso** [`phy_lmac_id`](lxdde/ports/iwlwifi/iwl_mvm_up.c) usaba
`IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT` (39) → LMAC 1 en 5 GHz.

**Linux** [`iwl_mvm_get_lmac_id`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/binding.c)
(`binding.c:168–173`) exige `IWL_UCODE_TLV_CAPA_CDB_SUPPORT` (40). AX200 cc-a0-77
no lo declara → LMAC 0 también en 5 GHz.

El REMOVE de PHY ch40 con `lmac_id=1` no alcanza el contexto creado en ch6 con
LMAC 0 → timeout y `MVM parado`.

## Orden de corrección (implementado)

1. **`phy_lmac_id`:** LMAC 5 GHz solo con capa **40** (CDB), no 39 (BINDING_CDB).
   BINDING_CDB (39) sigue gobernando payload CDB y REMOVE+ADD de banda.
2. **Hostcheck `cdb_lmac_test.c`:** bits 39/40 en cc-a0-77; assoc ch6→ch40 con
   `lmac_id=0` en BINDING/PHY REMOVE+ADD.
3. **Este informe + `hw-matrix.json`** (run4; assoc `fail` id=0x08 hasta validar fix).

## Validación en placa (usuario)

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

Esperado tras `wifi connect Rutilo`: sin `timeout … id=0x08`; siguiente HCMD =
MAC_CONTEXT / ADD_STA / AUTH TX.

---

# ROG run5 — ADD_STA y `ask` (13 sep 2026, ~17:30)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-13/` y `target/usb-diagnostic-2026-09-13-run5/`
(no al git). Consola de la foto cruzada con el SOSOLOG (el PSK de
`wifi connect` no se copia aquí).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`7483ccd71-dirty`)** (`SOSOHASH` 13-sep 15:18) |
| Hardware | `10de:249c` GA107 + `8086:2723` AX200 + `10ec:8168` rtl8169 |
| Userspace | **`sosh —`** + OTA `pid=2 write=182ms` |
| Teclado | **`kbd sc=127 enc=63 ent=63`**; `keyboard ready on slot=2 (0x0b05:0x18c6)` + `SET_PROTOCOL(Boot)` |
| GPU bring-up | `GSP_INIT_DONE`, `pool VRAM=sí`, CE readback, `compute listo cls=0xc7c0` sm_86 |
| WiFi scan | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, `SCAN_CFG v5` `add_sta_ver=12`, scan `count=21` |
| WiFi assoc | PHY ch3 **ok** → MAC `id=0x28` ok → **`timeout cmd grp=1 id=0x18`** (`ADD_STA`) |
| `ask hola` | mistral-7b **listo** → `generar rc=1` (doorbell CE motor 11; no es el fallo) |
| Halt | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` |
| SOSOWIFI | vacío (credenciales por sosh) |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/sta.c:128–157` (`iwl_mvm_sta_send_to_fw`), `fw/api/sta.h:18–64`, `mvm/fw.c:1781–1793` |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf` (doorbell/CE; no causal de `ask`) |
| `lxdde/reference/open-gpu-kernel-modules-570.144/` | tag **570.144** |

Hostchecks (no validan ADD_STA en silicio ni `ask` en placa): `l6-iwl-fw-hostcheck.sh` OK
(incl. `assoc_abi_test` y `cdb_lmac_test`), `l6-g3-gsp-hostcheck.sh` OK.

Fix CDB LMAC del run4 **validado**: ya no hay `timeout … id=0x08`.

## Tabla de etapas (run5 = flush #35 @ 290181 ms)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA `pid=2 write=182ms` | **OK** |
| Teclado USB 18c6 | `kbd sc=127 enc=63 ent=63`, SET_PROTOCOL Boot slot=2 | **OK** |
| GSP / RPC | `GSP_INIT_DONE`, `pool VRAM=sí` | **OK** |
| CE / compute | `CE readback verificado`, `compute listo cls=0xc7c0` | **OK** (bring-up) |
| `ask` mistral-7b | `mistral-7b listo` → `askd: generar rc=1` | **FAIL** |
| GSP fini | `GSP-RM apagado (… dma=off)` | **OK** |
| WiFi scan | `scan fin count=21` | **OK** |
| WiFi assoc | `timeout … id=0x18` (`ADD_STA`) | **FAIL** |
| Ethernet | `rtl_hw_start_8168h_1 ok`; `phystatus 0x84` DOWN | **FAIL** (sin cable) |
| DHCP | `net: dhcp…` sin lease | **pendiente** |

## Hallazgos

### WIFI-1. CDB LMAC del run4 ya no bloquea (confirmado, cerrado)

**Evidencia:** `PHY_CONTEXT ch3 band=1 action=2 ok` y RX `grp=1 id=0x08 seq=0x0010`.
El timeout pasó de PHY `id=0x08` (run4) a ADD_STA `id=0x18`.

### WIFI-2. `ADD_STA` pone `CLASS_AUTH|ASSOC` en el ADD, Linux no (confirmado)

**Síntoma:** `timeout cmd grp=1 id=0x18 slot=18; MVM parado` → `ADD_STA falló`.
MAC_CONTEXT (`id=0x28`) acababa de responder. El firmware no contesta el HCMD.

**soso** [`iwl_mvm_add_sta_ap`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c) (`iwl_mvm_assoc.c:150–169`):
`station_flags` = `STA_FLG_FAT_EN_40MHZ | MIMO_SISO | CLASS_AUTH | CLASS_ASSOC`
con la máscara incluyendo AUTH+ASSOC; `sta_id=1`; payload 48 B (v12).

**Linux** [`iwl_mvm_sta_send_to_fw`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/sta.c)
(`sta.c:128–157`):
`station_flags_msk` = `FAT_EN | MIMO_EN | RTS_MIMO_PROT` **sin** CLASS.
`STA_FLG_CLASS_AUTH` / `ASSOC` (`sta.h:18–19`, `sta.h:63–64`) significan
«la estación **ya** está autenticada/asociada». En 6.6.32 **ningún** `.c` de
iwlwifi las escribe en el ADD. El AP de un vif STA recibe `sta_id=0`
(`sta.c:37–40`); FAT 40 MHz solo si el STA declara ≥40 MHz (`sta.c:162–177`).

Compatible con el síntoma: el FW puede ignorar o no completar un ADD con
CLASS_ASSOC antes del 4-way. El hostcheck `assoc_abi_test.c` **exige** hoy esas
flags: hay que alinearlo a Linux al quitarlas.

### WIFI-3. `sta_id=1` y FAT 40 MHz fijos (hipótesis, secundario)

Linux reserva el AP en **sta_id 0**. soso usa `IWL_MVM_AP_STA_ID=1`.
`SCAN_CFG v5 bcast=0 add_sta_ver=12` (SOSOLOG) coincide con Linux
(`fw.c:1781–1793`: con API v12 **no** se añade aux STA). FAT_EN_40MHZ en un BSS
2.4 GHz canal 3 (20 MHz típico) no lo pone Linux.

No basta para explicar el timeout por sí solo; se corrige con WIFI-2.

### ASK-1. `ask: error` no es el doorbell nouveau (confirmado)

**Foto y SOSOLOG:** `askd: mistral-7b listo` → `generando (11 tokens… máx 128)` →
`canal (doorbell): runlist del motor 11 — PTOP … 0xc00400 (coinciden)` →
`askd: generar rc=1`.

Esa línea de doorbell la imprime `chan_refresh_doorbell_kick`
([`gsp_chan.c:680`](lxdde/ports/nouveau/gsp_chan.c)) en **cada** submit. Motor 11
es COPY0/CE (subida de pesos), no GR0 (motor 1). En el arranque aparece muchas
veces **con** CE readback OK. No hay `el QMD no señalizó`, ni
`offload GPU desactivado`, ni `mmap-fault` / babble.

`generar_tokens` ([`user/soso-llm/src/main.rs:1065–1074`](user/soso-llm/src/main.rs))
devuelve 1 en `Err(())` **sin** `print_diagnostics` (askd llama con
`verboso=false`). La causa exacta del `Err` (tensor, capa, GPU) **no está en el
log**.

### ASK-2. Primera subida CE de mistral y `Err` mudo (hipótesis)

Bring-up Ampere llegó a compute sm_86 y el modelo cargó. El único kick durante
`ask` es CE. Hipótesis: `gpu_map`/subida del primer tensor arranca el CE y
después `generate_stream_par` falla en CPU/GPU sin mensaje. Hay que imprimir
capa/tensor/`last_fail` antes de teorizar QMD Ampere.

## Orden de corrección

1. **`iwl_mvm_add_sta_ap`:** quitar `CLASS_AUTH|ASSOC` del ADD (Linux `sta.c:136–138`).
   `sta_id=0` como el AP de un vif STA. FAT/MIMO: 20 MHz SISO salvo HT40 en el
   beacon. Hostcheck `assoc_abi_test`: no exigir CLASS en el ADD.
   *Host:* `./scripts/l6-iwl-fw-hostcheck.sh`. *Placa:* `wifi connect` sin
   `timeout … id=0x18`; siguiente HCMD = TIME_EVENT / AUTH TX.
2. **Log de `ask`:** en `generar_tokens` / askd imprimir el `Err` (capa, tensor,
   `SysGpu::last_fail`). Sin eso el siguiente ciclo sigue ciego.
   *Host:* `cargo xtask test` shard llm no debe perder el mensaje. *Placa:*
   `ask hola` deja una línea de causa en SOSOLOG, no solo `generar rc=1`.
3. **Tras el log, Ampere CE/matvec de mistral-7b** si la causa es subida/submit:
   `gsp_buf_upload` / QMD sm_86 vs el `Err` nuevo. No tocar GSP bring-up (GO).
4. **Este informe + `hw-matrix.json`** (run5; assoc `fail` id=0x18; `carga_real`
   `fail`; racha de arranques **no** incrementada).

## Qué no se ha hecho

- No se ha tocado el driver ni reflasheado.
- No hay lease DHCP ni SSH en placa (assoc no completa; ethernet DOWN).
- El doorbell de la foto no se ha silenciado (es diagnóstico de submit, no el bug).

---

# ROG run6 — SESSION_PROTECTION AX200 (13 sep 2026, ~18:30)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-13-run6/` (mismo SOSOLOG que
`target/usb-diagnostic-2026-09-13/`).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`34eff4aa1-dirty`)** |
| Hardware | `10de:249c` GA107 + `8086:2723` AX200 + `10ec:8168` rtl8169 |
| Userspace | **`sosh —`** + OTA `pid=2 write=164ms` |
| Teclado | **`kbd sc=167 enc=83 ent=83`** |
| GSP | `GSP_INIT_DONE`, `pool VRAM=sí`, CE readback, `compute listo cls=0xc7c0` sm_86 |
| WiFi scan | `UCODE_ALIVE_NTFY`, scan `count=23` |
| WiFi assoc | PHY ch3 ok → MAC ok → **ADD_STA ok** → **`timeout cmd grp=1 id=0x29`** |
| `ask hola` | mistral-7b listo → `generar rc=1` (sin línea de causa en SOSOLOG) |
| Ethernet | `phystatus 0x84` DOWN (sin cable) |

Árboles Linux: **`lxdde/linux/` 6.6.32** — `mac80211.c:2471–2477`,
`time-event.c:1185–1237`, `fw/file.h:418` (capa 54).

Hostchecks: `./scripts/l6-iwl-fw-hostcheck.sh` OK (SESSION_PROT v1, TIME_EVENT ausente en cc-a0-77).

## Tabla de etapas (run6 = flush #39 @ 295682 ms)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA ok | **OK** |
| Teclado | `kbd sc=167 enc=83 ent=83` | **OK** |
| GSP / RPC | `GSP_INIT_DONE`, pool VRAM, compute sm_86 | **OK** |
| WiFi scan | `scan fin count=23` | **OK** |
| WiFi assoc | ADD_STA ok → `timeout … id=0x29` (TIME_EVENT) | **FAIL** |
| `ask` | `generar rc=1` sin capa/tensor en log | **FAIL** |
| Ethernet | `phystatus 0x84` DOWN | **FAIL** (sin cable) |

## Hallazgo confirmado — TIME_EVENT vs SESSION_PROTECTION

**Síntoma:** tras ADD_STA (`grp=1 id=0x18` ok), `timeout cmd grp=1 id=0x29`.

**soso** [`iwl_mvm_protect_assoc`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c): mandaba
`TIME_EVENT_CMD` 0x29 LEGACY.

**Linux** [`iwl_mvm_protect_assoc`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mac80211.c):
con `IWL_UCODE_TLV_CAPA_SESSION_PROT_CMD` (54) usa
`SESSION_PROTECTION_CMD` grp=3 id=0x5, no TIME_EVENT.

**Ucode cc-a0-77:** capa 54 sí; CMD_VERSIONS grp=3 cmd=0x5 ver=1; **no** TIME_EVENT 0x29.

Fix run5 (ADD_STA sin CLASS_AUTH|ASSOC) **validado** en run6.

## Fixes aplicados (código)

1. **`iwl_mvm_protect_assoc`:** si capa 54 → `SESSION_PROTECTION_CMD` (24 B,
   `SESSION_PROTECT_CONF_ASSOC`, 900 ms → TU); si no → TIME_EVENT legacy.
2. **Hostcheck:** `assoc_abi_test` + parse cc-a0-77 (capa 54, no 0x29).
3. **`askd`:** log explícito capa/tensor/gpu en fallo de inferencia.

## Validación en placa (usuario)

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
# Para ask con log nuevo también rootfs (soso-llm/askd):
# cargo xtask flash-usb-live /dev/sda --yes --skip-models
```

Esperado: `wifi connect` sin `timeout … id=0x29`; log
`SESSION_PROTECTION CONF_ASSOC ok`; siguiente HCMD = TX AUTH.
`ask hola` debe dejar `askd: inferencia falló capa …` si sigue fallando.

## Qué no se ha hecho

- No reflasheado ni validado en placa en este ciclo.
- CE/QMD mistral Ampere: pendiente de la línea de log en placa.

---

## Run7 — AUTH TVQM y doorbell Ampere (flush #763 @ 4319649 ms)

Copias: `target/usb-diagnostic-2026-09-13-run7/`. Kernel **0.2.2 (`5e48a396c-dirty`)**.

| Etapa | SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA ok | **OK** |
| Teclado | `keyboard ready` 1866 + 18c6; `kbd sc=206 enc=101 ent=101` | **OK** |
| GSP | `GSP_INIT_DONE`, pool VRAM, CE readback, compute sm_86 | **OK** |
| `ask mistral` | **`generar rc=0`** | **OK** |
| WiFi scan | `UCODE_ALIVE_NTFY`, scan count=25/21 | **OK** |
| WiFi assoc | `SESSION_PROTECTION CONF_ASSOC ok (878 TU)` → **`AUTH timeout`** | **FAIL** |
| SOSOLOG | **364** líneas `canal (doorbell)… motor 11` durante `ask` | **FAIL** |

### Hallazgo — AUTH por cola HCMD

Tras ADD_STA y SESSION_PROT, **cero RX AUTH**. `iwl_mvm_tx_mgmt` enviaba
`TX_CMD` por `iwl_trans_send_cmd_async` → cola HCMD q0. Linux AX200 (gen2)
usa cola mgmt TVQM (`SCD_QUEUE_CFG` 0x1d, tid=15) + `iwl_trans_tx`.

### Hallazgo — doorbell Ampere en hot path

`chan_refresh_doorbell_kick` releía PTOP y logueaba en **cada** submit CE/QMD
(364× en un `ask`), llenando el ring de 256 KiB.

### Fixes aplicados (código, pendiente placa)

1. **`iwl_trans_txq_alloc_mgmt` + `iwl_trans_tx`:** DMA TFD/BC, `SCD_QUEUE_CFG`
   tras ADD_STA; AUTH/ASSOC por cola de datos (`doorbell qid≠0`).
2. **`gsp_chan.c`:** resolución PTOP Ampere solo en `chan_start`; submit usa
   kick cacheado.
3. **Hostcheck:** `assoc_abi_test` exige SCD_QUEUE_CFG y TX fuera de HCMD.
4. **Matriz:** `ga107-igpu` / `ax200-wifi` actualizados con run7.

### Validación en placa

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
wifi connect Rutilo   # sin AUTH timeout; RX AUTH seq=2
ask mistral-7b        # SOSOLOG sin spam doorbell
```

---

## Run8 — SCD_QUEUE_CFG BC 32 B y inferencia lenta (flush #47 @ 875643 ms)

Copias: `target/usb-diagnostic-2026-09-13/` (ESP `/dev/sda1`). Kernel **0.2.2
(`c5cc1ff81-dirty`)**. Comparado con Linux **v6.6** (`queue/tx.c`, `iwl-fh.h`).

| Etapa | SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, teclado `kbd sc=103` | **OK** |
| GSP/CE | `GSP_INIT_DONE`, pool VRAM, compute sm_86 | **OK** |
| WiFi scan | count=23 | **OK** |
| WiFi assoc | ADD_STA ok → **`timeout cmd grp=1 id=0x1d`** → `SCD_QUEUE_CFG mgmt falló` | **FAIL** |
| `ask hola` | mistral-7b listo, backend GPU, 16 puntos @ ~875 s; sin `QMD no señalizó`, sin spam doorbell | **FAIL** (lento; sin tok/s ni fin) |

### Hallazgo — BC tabla TXQ mgmt 32 B vs 640 B

Tras run7 (TVQM), el AUTH ya no va por HCMD pero **`SCD_QUEUE_CFG` (0x1d)**
expira: soso asignaba `byte_cnt` de **32 B** (`16 × u16`); Linux con
`queue_alloc_cmd_ver==0` usa **`iwlagn_scd_bc_tbl` = 640 B** (pool DMA alineado
256). Firmware cc-a0-77: `0x1d` no está en LEGACY/DATA_PATH CMD_VERSIONS → ruta
legado correcta, BC mal dimensionada.

### Hallazgo — pesos lazy desde USB p3

`fijar_pesos_residentes` solo fijaba el flag; cada capa del prefill subía
shards mmap (294) en el primer matvec → minutos por punto de progreso aunque
GSP/CE estuvieran bien.

### Fixes aplicados (código, pendiente placa)

1. **`iwl_trans.c` / `iwl_internal.h`:** BC **640 B** (`TFD_QUEUE_BC_SIZE`),
   alineación DMA 256, duplicado BC en `TFD_QUEUE_SIZE_MAX+idx`.
2. **`assoc_abi_test.c`:** payload 24 B, `cb_size` 16 TFD, `byte_cnt` 640 B.
3. **`soso-gpu` / `soso-llm`:** subida eager Q4/Q8 a VRAM tras cargar modelo;
   log `subida eager VRAM`; telemetría `capa i/n ms on_gpu=`; sin `sleep_ms(1)`
   en `ask_layer_tick`.
4. **CE/QMD Ampere (+64 / tiles G5):** no tocado — este SOSOLOG no muestra
   `on_gpu=0` ni `bounce>0`; pendiente de la línea de log tras flash.

### Validación en placa

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
# ask con log nuevo también rootfs:
# cargo xtask flash-usb-live /dev/sda --yes --skip-models
wifi connect Rutilo   # sin timeout id=0x1d; log TXQ mgmt qid≠0
ask hola              # línea subida eager antes del primer '.'; tok/s al terminar
```

---

# ROG run9 — SCD_QUEUE_CFG TFD vacíos (13 sep 2026, flush #77 @ 1527382 ms)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias:
`target/usb-diagnostic-2026-09-13-run9/` (también
`target/usb-diagnostic-2026-09-13/` de esta misma pasada). ESP desmontada.
SOSOWIFI vacío (PSK no copiado; `wifi connect` interactivo).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`c5cc1ff81-dirty`)** |
| Hardware | `10de:249c` GA107 + `8086:2723` AX200 + `10ec:8168` rtl8169 |
| Userspace | **`sosh —`** + OTA `pid=2 write=183ms` + `$ halt` |
| Teclado | **`kbd sc=99 ultimo=0x1c enc=49 ent=49`**; `SET_PROTOCOL(Boot)` slot 2 `1866` y slot 3 `18c6` |
| GPU | `GSP_INIT_DONE res=0x0`, `pool VRAM=sí`, CE readback, `ask hola` **`generar rc=0`** `backend GPU`, fini `unload=ok dma=off` |
| WiFi scan | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE`, scan `count=24` luego `25` |
| WiFi assoc | PHY ch3 ok → MAC `0x28` ok → ADD_STA `0x18` ok → **`timeout cmd grp=1 id=0x1d`** → `SCD_QUEUE_CFG mgmt falló` |
| Ethernet | `rtl_hw_start_8168h_1 ok`; `phystatus 0x84` DOWN (sin cable) |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `queue/tx.c:123–129, 1047–1108, 1247–1262`, `pcie/trans.c:2028–2044`, `iwl-fh.h:663–729`, `fw/api/txq.h:82–115`, `mvm/sta.c:852–891`, `mvm/mvm.h:1450–1454` |

Hostchecks (no validan SCD DMA en silicio): `l6-iwl-fw-hostcheck.sh` OK,
`l6-g3-gsp-hostcheck.sh` OK. `assoc_abi_test` mockea HCMD; no cubre TFD
`invalid_tx_cmd` ni el timeout de placa.

---

## Tabla de etapas (run9)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA, `$ halt` | **OK** |
| Teclado USB 18c6 | `keyboard ready on slot=3 (0x0b05:0x18c6)`, `sc=99` | **OK** |
| GSP / RPC | `GSP_INIT_DONE`, `pool VRAM=sí` | **OK** |
| CE / compute | `CE readback verificado`, `compute listo cls=0xc7c0` | **OK** |
| `ask hola` | `backend GPU`, `generar rc=0` | **OK** (sin tok/s en log) |
| Apagado GSP | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`, `alive=true` | **OK** |
| WiFi scan | `scan fin count=24` / `25` | **OK** |
| WiFi assoc | `timeout … id=0x1d` / `TXQ mgmt falló` | **FAIL** |
| DHCP / SSH | ethernet DOWN; sin lease WiFi | **pendiente** |

---

## Hallazgos

### WIFI-1. `SCD_QUEUE_CFG` (0x1d) sigue sin respuesta tras ADD_STA (confirmado)

**Síntoma:** `iwl_rx: grp=1 id=0x18 seq=0x0011` (ADD_STA) → ningún RX de `0x1d` →
`timeout cmd grp=1 id=0x1d slot=18; MVM parado` → `SCD_QUEUE_CFG mgmt falló` →
`fallo AUTH+ASSOC`. El FW no NACKea: no contesta.

**soso:** [`iwl_trans.c:957–1012`](lxdde/ports/iwlwifi/iwl_trans.c) asigna TFD 16×256 B,
BC 640 B, `cb_size=TFD_QUEUE_CB_SIZE(16)`, payload 24 B
`iwl_tx_queue_cfg_cmd`, HCMD LEGACY→LONG grupo 1. [`iwl_mvm_assoc.c:467`](lxdde/ports/iwlwifi/iwl_mvm_assoc.c)
tras ADD_STA (`sta_id=0`, tid=15). `memset` de los TFD a **cero**.

**Linux 6.6.32:** AX200 es gen2 → `iwl_mvm_has_new_tx_api` ([`mvm.h:1450`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/mvm.h))
usa TVQM: `iwl_txq_dyn_alloc` con `queue_alloc_cmd_ver==0` envía el mismo
`SCD_QUEUE_CFG` 24 B ([`queue/tx.c:1247–1262`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/queue/tx.c)).
Mgmt: `IWL_MGMT_QUEUE_SIZE=16` ([`txq.h:82`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/txq.h)).
BC `iwlagn_scd_bc_tbl` = 640 B ([`iwl-fh.h:727`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-fh.h)).
cc-a0-77 **sin** `CAPA_DQA` (hostcheck): Linux tampoco manda `DQA_ENABLE`.

**Compatible con el síntoma:** sí. El 640 B del run8 no basta: el HCMD sigue
expirando. Siguiente discrepancia causal (etapa ausente):

Linux, **antes** de enviar `SCD_QUEUE_CFG`, rellena cada TFD con
`invalid_tx_cmd` (DMA real, no dirección 0):

- alloc: [`pcie/trans.c:2028–2044`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/trans.c)
  (`INVALID_WR_PTR_CMD` / `DEBUG_GROUP`)
- init TFD: [`queue/tx.c:1102–1108`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/queue/tx.c) +
  [`queue/tx.c:123–129`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/queue/tx.c)
  (`iwl_txq_set_tfd_invalid_gen2` → TB0 = `invalid_tx_cmd.dma`)

soso no tiene `invalid_tx_cmd`. TFD a ceros ⇒ TB0 `addr=0`. Si el FW prefetcha
el anillo al procesar `0x1d`, DMA desde 0 cuelga el HCMD (timeout, no NACK).

### GPU-1. Inferencia Ampere en placa (confirmado OK)

`soso-llm: backend GPU` + `askd: generar rc=0`. No hay línea `tok/s` ni
`subida eager VRAM` (userspace del USB puede ser anterior al log nuevo). No es
bloqueante para assoc.

### ETH-1. 8168H DOWN (descartado como bug de este arranque)

`rtl_hw_start_8168h_1 (VER_46) ok` + `phystatus 0x84` sin cable. No mezclar con
DHCP WiFi.

---

## Orden de corrección

1. **`iwl_trans.c`:** alocar `invalid_tx_cmd` (Linux `pcie/trans.c:2028`) e
   inicializar los 16 TFD mgmt con TB0 a esa DMA **antes** de `SCD_QUEUE_CFG`
   (`queue/tx.c:1102–1108`). Log `SCD_QUEUE_CFG tfd= bc= cb_size= n=`.
   Host: `assoc_abi_test` exige TB0 ≠ 0. Placa: `TXQ mgmt qid≠0` sin timeout
   `id=0x1d`; siguiente HCMD = `SESSION_PROTECTION`.
2. **Si 0x1d sigue expirando:** volcar `tfdq_addr`/`byte_cnt_addr` físicos del
   HCMD y contrastar con el anillo HCMD (`BA=0x21242000` en este log). No
   cambiar el ABI 24 B / BC 640 B / cola 16 sin esa evidencia.
3. **Este informe + `hw-matrix.json`** (`ga107-igpu` / `ax200-wifi`, run9).

## Qué no se ha hecho

- Ni `iwl_trans.c` ni `assoc_abi_test` tocados en este ciclo.
- No reflasheado. No 4-way, DHCP WiFi ni SSH en placa.
- Ethernet sin cable. GB205/AX211 ausentes en este hwscan.

