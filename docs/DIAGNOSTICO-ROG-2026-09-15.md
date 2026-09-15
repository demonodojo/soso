# ROG AX200: AUTH timeout tras SCD/TLC OK (15 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat, extraíble) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-15/` (ESP desmontada). Log útil recortado en
`SOSOLOG-clean.txt` (~957 líneas; el `.TXT` de 256 KiB tiene relleno de
newlines). Sin PSK en este informe (`SOSOWIFI.TXT` vacío; el connect fue manual).

| Campo | Valor (flush **#32**, último arranque) |
|---|---|
| Kernel empaquetado (SOSOHASH) | **0.2.2 (`dfec53f84-dirty`)** |
| Kernel en placa (SOSOLOG) | **0.2.2 (`5a7be1388-dirty`)** — más nuevo que el USB |
| Flush | **#32 @ 162569 ms** (~163 s) |
| Hardware | `10de:249c` + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2) |
| WiFi | ALIVE + MVM + scan **23 BSS**; **assoc FAIL** (AUTH timeout **ch3**) |
| GPU | GSP_INIT_DONE + VRAM pool + compute sm_86 + apagado limpio |
| Ethernet | rtl8169 enlace DOWN (sin cable) → sin DHCP/SSH por cable |

Flush anterior documentado: **#31 @ 147436 ms** (`dfec53f84-dirty`), AUTH timeout ch40.

Árboles Linux (solo lectura):

| Árbol | Tag / commit | Uso |
|---|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** | `phy-ctxt.c` RLC_CONFIG, `tx.c` offload_assist, `time-event.h` SESSION_PROT |

Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK** (incluye RLC tras PHY y
TX AUTH `offload_assist=0x0c00`). Firmware `iwlwifi-cc-a0-77.ucode`:
`IWL_UCODE_TLV_CAPA_TLC_OFFLOAD` **bit 43 = 1**; `RLC_CONFIG_CMD` **ver 3**;
`PHY_CONTEXT` **ver 4**.

---

## Tabla de etapas (flush #32 — kernel `5a7be1388`)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (5a7be1388-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#32** @ 162 s | OK |
| GPU GSP | `GSP_INIT_DONE`; pool VRAM=sí; compute sm_86; `GSP-RM apagado … dma=off` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; NVM/MCC; `up mínimo listo` | **OK** |
| Scan userspace | `SCAN_COMPLETE count=23`; Rutilo WPA2 **ch3** | **OK** |
| Connect | `wifi connect Rutilo …`; PHY ch3 band=1 MODIFY | parcial |
| ADD_STA | `ADD_STA status=0x00000001` | **OK** |
| TLC / SCD / TXQ | `TLC_MNG_CONFIG`; `SCD wide ver_hdr=0 ver_tlv=3` → `TXQ mgmt qid=1` | **OK** |
| SESSION_PROT | `SESSION_PROTECTION CONF_ASSOC ok (878 TU)` | **OK** |
| PHY ch3 | `PHY_CONTEXT ch3 band=1 action=2 ok` (sin log RLC posterior) | parcial |
| TX AUTH | `tx qid=1 doorbell=0x00010001 seq=0x0100 len=50` | enviado |
| TX resp | **cero** `iwl_trans: TX resp …` ni `iwl_rx … id=0x1c` en todo el log | **FAIL** |
| RX datapath | **cero `REPLY_RX_MPDU`** tras PHY ch3 | **FAIL** |
| AUTH / 4-way / DHCP | `AUTH sin retry (tx status=0x00)` → `AUTH timeout` | **FAIL** / pendiente |

Secuencia:

```
CMD_VERSIONS + ALIVE → INIT_COMPLETE → MVM up
→ scan 23 BSS (Rutilo WPA2 ch3)
→ ADD_STA status=0x1 → TLC_MNG_CONFIG → SCD ver_tlv=3 → TXQ mgmt qid=1
→ PHY_CONTEXT ch3 → SESSION_PROTECTION CONF_ASSOC ok
→ tx qid=1 len=50 (doorbell ok)
→ sin TX_CMD 0x1c / last_mgmt_tx_status=0x00 → AUTH timeout
```

Cabecera del flush: `ultimo=0x1c enc=40 ent=40` — el ring de log registró un
opcode 0x1c al cerrar, pero **no** aparece en la traza `lx:` parseada; el driver
nunca imprimió `TX resp` ni actualizó `last_mgmt_tx_status` a `0x01`.

---

## Tabla de etapas (flush #31)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (dfec53f84-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#31** @ 147 s | OK |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; `alive=true` | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; NVM/MCC/SCAN_CFG; `up mínimo listo` | **OK** |
| Scan userspace | `wifi scan` → **26 BSS**, Rutilo WPA2 **ch40** | **OK** |
| Connect | `wifi connect Rutilo …`; rescan; PHY ch40 band=0 | parcial |
| ADD_STA | `ADD_STA status=0x00000001` (SUCCESS) | **OK** |
| TLC / SCD / TXQ | `TLC_MNG_CONFIG AP sta_id=0 async`; `SCD … len=8` → `TXQ mgmt qid=1` | **OK** |
| PHY ch40 | `PHY_CONTEXT ch40 band=0` (sin RLC_CONFIG posterior) | parcial |
| RX datapath | **cero `REPLY_RX_MPDU`** tras PHY ch40 (ni beacons Rutilo) | **FAIL** |
| AUTH TX | `tx qid=1 … TX resp status=0x01`; retry `IN_CONTROL_AND_MGMT` | enviado, sin respuesta |
| AUTH / 4-way / DHCP | `AUTH timeout`; sin 4-way ni DHCP | **FAIL** / pendiente |

Secuencia:

```
CMD_VERSIONS +192 → ALIVE → INIT_COMPLETE → MVM up
→ scan 26 BSS (Rutilo WPA2 ch40)
→ ADD_STA status=0x1 → TLC_MNG_CONFIG → SCD → TXQ mgmt qid=1
→ PHY_CONTEXT ch40 (rxchain en PHY; sin RLC_CONFIG grp=5 id=0x8)
→ AUTH TX status=0x01 (offload_assist=0)
→ cero REPLY_RX_MPDU → AUTH timeout
```

---

## Hallazgos (flush #32)

### WIFI-7. Respuesta TX_CMD ausente (bloqueante AUTH)

**Síntoma:** tras doorbell AUTH (`tx qid=1 … len=50`) no hay ninguna línea
`iwl_trans: TX resp frame_count=… status=0x01`. `last_mgmt_tx_status` queda en
**0x00**; el retry ctl-filter aborta (`AUTH sin retry ctl-filter`).

**Contraste flush #31:** en #31 sí había `TX resp status=0x01` pero cero RX;
en #32 el FW **no confirma** la transmisión (o la notificación no llega al parser).

**soso:** [`iwl_trans.c`](../lxdde/ports/iwlwifi/iwl_trans.c) `parse_tx_resp` solo
si `group == LEGACY_GROUP && cmd == TX_CMD` (0x1c). Tras doorbell solo se drenan
8 iteraciones RX (`iwl_trans_tx`); `wait_mlme_flag` hace 4×poll + 20 ms × N.

**Linux 6.6:** [`ops.c:309`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/ops.c)
handler `RX_HANDLER(TX_CMD, iwl_mvm_rx_tx_cmd)` en LEGACY_GROUP; el op_mode espera
la respuesta antes de continuar MLME.

**Hipótesis ordenadas:**

1. Notificación `TX_CMD` en grupo distinto de LEGACY (0) — ampliar filtro y log.
2. Cola mgmt no totalmente operativa tras SCD ADD (falta paso enable/drenaje
   equivalente a `iwl_mvm_tvqm_enable_txq` post-ADD_STA en Linux [`sta.c:2220-2229`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/sta.c)).
3. Poll RX insuficiente entre doorbell y timeout AUTH.
4. Offset status TX gen2: soso usa `IWL_MVM_TX_RESP_V3_STATUS_OFF=36` cuando
   `!gen3` — verificar contra struct Linux para AX200 22000.

### WIFI-8. RX sordo tras PHY ch3 (secundario hasta TX success)

Tras `PHY_CONTEXT ch3` no hay `REPLY_RX_MPDU` (ni beacons Rutilo). Puede ser
consecuencia de TX no confirmado o de RLC_CONFIG silencioso sin efecto. En
kernel `5a7be1388` el código RLC ya está en árbol ([`iwl_mvm_up.c`](../lxdde/ports/iwlwifi/iwl_mvm_up.c)
`iwl_mvm_phy_send_rlc`) pero **no loguea éxito**.

---

## Hallazgos (flush #31)

### WIFI-4. Falta RLC_CONFIG tras PHY_CONTEXT (bloqueante RX)

**Síntoma:** tras `PHY_CONTEXT ch40` no hay `REPLY_RX_MPDU` (ni beacons del AP).
El scan UMAC previo no usa este PHY; el AUTH sí.

**soso:** [`iwl_mvm_up.c`](../lxdde/ports/iwlwifi/iwl_mvm_up.c) `phy_ctxt_apply`
(ver ≥ 3) mete `rxchain_info` en PHY y **nunca** envía `RLC_CONFIG_CMD`.

**Linux 6.6** [`phy-ctxt.c:149-186`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/phy-ctxt.c):
si `RLC_CONFIG` ≥ 2, **no** pone cadenas en PHY y tras ADD/MODIFY envía
`RLC_CONFIG_CMD` (`datapath.h` id **0x8**, struct v2 `phy_id` + `rlc.rx_chain_info`).
El blob AX200 es **ver 3**.

**Compatible con el síntoma:** PHY sin RLC deja el RX sordo en el canal de assoc.

### WIFI-5. TX AUTH sin `offload_assist` MH_SIZE (bloqueante trama)

**Síntoma:** AUTH TX `status=0x01` no acredita que el AP entendiera la trama.

**soso:** [`iwl_mvm_assoc.c`](../lxdde/ports/iwlwifi/iwl_mvm_assoc.c) `iwl_mvm_tx_mgmt`
deja `offload_assist=0`.

**Linux 6.6** [`tx.c:128-135`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/tx.c):
`mh_len/2 << TX_CMD_OFFLD_MH_SIZE` (bit 8). AUTH 24 B → **12 << 8 = 0x0c00**.
Sin eso el FW no ve la cabecera 802.11.

### WIFI-6. SESSION_PROTECTION log conf_id (no bloqueante)

El 4.º DW del notif es `conf_id` (Linux [`time-event.h:412-417`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/time-event.h)),
no duración. El `duration=0 TU` del log era `CONF_ASSOC=0`. Corregido en
[`iwl_trans.c`](../lxdde/ports/iwlwifi/iwl_trans.c).

---

## Historial: SCD timeout con LQ_CMD (flushes #30 / #32)

Arranques previos del mismo USB mostraban otro bloqueo ya corregido en árbol:

| Flush | LQ_CMD | SCD / TXQ | Bloqueo |
|---|---|---|---|
| #30 @ 538 s | **sí** | `timeout grp=5 id=0x17` | SCD |
| #32 @ 135 s | **sí** | `timeout grp=5 id=0x17`; cero RX | SCD |
| **#31 @ 147 s** | **no** (TLC) | `SCD … len=8` → `TXQ mgmt qid=1` | **AUTH / RX** |

**WIFI-1 (LQ con TLC offload):** Linux [`utils.c:253-266`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/utils.c)
no envía LQ si capa 43; soso ya omitía LQ en flush #31. Fix TLC aplicado
antes de este ciclo (ver abajo).

Evidencia A/B (`2026-09-14-run3` sin LQ → SCD OK pero AUTH timeout aparte).

---

## Corrección aplicada en árbol (15 sep)

### Fase 1 — SCD / TLC (flushes #30/#32)

1. **`IWL_UCODE_TLV_CAPA_TLC_OFFLOAD` (43)** en `iwl_internal.h`.
2. **`iwl_mvm_rate_init_ap_sta`**: con TLC → `TLC_MNG_CONFIG_CMD` v4 async;
   sin TLC → `LQ_CMD` (8265).
3. **`iwl_mvm_add_sta_ap`**: lee `ADD_STA status=0x…`, aborta si ≠ SUCCESS.
4. Hostcheck assoc ABI sin LQ con TLC=1.

### Fase 2 — AUTH / RX (flush #31)

1. **`RLC_CONFIG_CMD`** (grp 5 id 0x8) en `iwl_internal.h`; tras PHY ADD/MODIFY
   si `cmd_ver >= 2`: `rxchain_info=0` en PHY y HCMD `iwl_rlc_config_cmd`.
2. **`iwl_mvm_tx_mgmt`**: `offload_assist = (24/2) << TX_CMD_OFFLD_MH_SIZE` (= `0x0c00`)
   en TX gen2/gen3 AUTH/ASSOC.
3. **`iwl_trans.c`**: log SESSION_PROTECTION con `conf_id`, no `duration`.
4. Hostcheck: PHY `rxchain_info==0`, RLC tras PHY, TX0 `offload_assist==0x0c00`.
   `./scripts/l6-iwl-fw-hostcheck.sh` verde.

Validación placa con kernel `5a7be1388` (fase 1+2 en árbol; flush #32 ya lo ejecutó
sin empaquetar en SOSOHASH):

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
# en placa: wifi connect Rutilo …
# esperado: iwl_trans: TX resp … status=0x01
#           REPLY_RX_MPDU (beacons) tras PHY ch3
#           rx AUTH seq=2 → mlme_auth_ok → 4-way → DHCP LxWifi
```

### Fase 3 — TX resp / cola mgmt (flush #32)

1. **Parse TX_CMD en cualquier grupo:** en `iwl_trans.c` loguear todo
   `cmd==0x1c` antes del filtro de grupo; aceptar también `LONG_GROUP (1)` si
   el FW lo usa en gen2 TVQM.
2. **Habilitar cola tras ADD_STA:** revisar orden Linux
   `iwl_mvm_tvqm_enable_txq` → `iwl_trans_txq_alloc` post-estación; comparar con
   `iwl_trans_txq_alloc_mgmt` en soso (SCD ADD puede no bastar sin enable).
3. **Poll TX resp durante AUTH:** en `iwl_mvm_mlme_auth_assoc` / `wait_mlme_flag`,
   drenar RX hasta recibir `TX_CMD` o timeout corto antes de juzgar
   `last_mgmt_tx_status`.
4. **Log RLC_CONFIG ok** tras PHY MODIFY en canal de assoc (confirmar en placa).
5. **RX mgmt** si TX success pero sin `rx AUTH seq=2`: filtros MAC
   `IN_CONTROL_AND_MGMT`, binding MAC↔PHY, `parse_rx_mpdu` → MLME.
6. **Reflash + matriz:** criterios anteriores; actualizar `docs/hw-matrix.json`.

---

## Qué no se ha hecho

Implementación de fase 3. Hostchecks no re-ejecutados en esta sesión.
`target/` no va al git. SOSOHASH del USB sigue en `dfec53f84` (desalineado del
kernel de placa `5a7be1388`).
