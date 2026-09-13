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

