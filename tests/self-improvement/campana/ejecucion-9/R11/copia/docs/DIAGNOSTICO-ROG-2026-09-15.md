# ROG AX200: AUTH timeout tras SCD/TLC OK (15 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat, extraíble) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-15/` (ESP desmontada). Sin PSK en este
informe (`SOSOWIFI.TXT` vacío; el connect fue manual).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`dfec53f84-dirty`)** |
| Flush | **#31 @ 147436 ms** (bloqueo AUTH; SCD/TLC ya OK) |
| Hardware | `10de:249c` + `8086:2723` (AX200) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2) |
| WiFi | ALIVE + MVM + scan 26 BSS; **assoc FAIL** (AUTH timeout ch40) |
| Ethernet | rtl8169 enlace DOWN (sin cable) → sin DHCP/SSH por cable |

Árboles Linux (solo lectura):

| Árbol | Tag / commit | Uso |
|---|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** | `phy-ctxt.c` RLC_CONFIG, `tx.c` offload_assist, `time-event.h` SESSION_PROT |

Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK** (incluye RLC tras PHY y
TX AUTH `offload_assist=0x0c00`). Firmware `iwlwifi-cc-a0-77.ucode`:
`IWL_UCODE_TLV_CAPA_TLC_OFFLOAD` **bit 43 = 1**; `RLC_CONFIG_CMD` **ver 3**;
`PHY_CONTEXT` **ver 4**.

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

Validación placa pendiente (kernel con fase 1+2):

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
# en placa: wifi connect Rutilo …
# esperado: REPLY_RX_MPDU (beacons) tras PHY ch40
#           rx AUTH seq=2 status=0 → mlme_auth_ok → 4-way → DHCP
```

---

## Qué no se ha hecho

Reflash del USB ni ciclo de placa con el kernel de fase 2. Sin `--boot-ok`.
`target/` no va al git. GPU de estos flushes no se re-diagnostica aquí
(GSP/VRAM/CE vistos OK).
