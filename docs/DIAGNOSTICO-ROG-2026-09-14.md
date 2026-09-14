# ROG: assoc AX200 — SCD_QUEUE_CONFIG_CMD v3 (14 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-14/` (ESP desmontada). SOSOWIFI vacío en el
arranque inicial (PSK introducida en sesión con `wifi connect`).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`c5cc1ff81-dirty`)** — no HEAD (`d3ab499c4`) |
| BOOTMARK | 14-sep ~08:45 |
| Hardware | `10de:249c` (RM GA107, 16 GiB) + `8086:2723` (AX200) + `10ec:8168` (rtl8169) |
| Userspace | **`sosh —`** + OTA `pid=2 write=166ms` + `$ halt` |
| Teclado | **`kbd sc=219 enc=108`**; `keyboard ready` slot 2 `1866` y slot 3 **`18c6`** |
| GPU | `GSP_INIT_DONE res=0x0`, `pool VRAM=sí`, CE, `ask hola` **`generar rc=0`** `backend GPU` |
| WiFi assoc | PHY/MAC/ADD_STA ok → **`timeout cmd grp=1 id=0x1d`** (×2) → `SCD_QUEUE_CFG mgmt falló` |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | **6.6.32** — `mvm/ops.c:1306–1312`, `queue/tx.c:1247–1276`, `fw/api/datapath.h:547–597`, `pcie/trans.c:2028–2044` |

Hostchecks: `l6-iwl-fw-hostcheck.sh` OK (incl. `assoc_abi_test` exige DATA_PATH `0x17` 36 B).

---

## Tabla de etapas (flush #94 @ 2908508 ms)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Userspace | `sosh —`, OTA `write=166ms`, `$ halt` | **OK** |
| Teclado USB 18c6 | `SET_PROTOCOL(Boot)` ×2; `keyboard ready on slot=3 (0x0b05:0x18c6)`; `sc=219` | **OK** |
| GSP / RPC | `GSP_INIT_DONE`, `pool VRAM=sí` | **OK** |
| CE / compute | `CE readback verificado`, `compute listo cls=0xc7c0` | **OK** |
| G6 (este USB) | `techo residente ~8192 MiB de 16068` (`familia=ga107`) | **pendiente HEAD** (32 GiB en `d3ab499c4`) |
| `ask hola` | `backend GPU`, `generar rc=0` | **OK** |
| Apagado GSP | `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)` | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`, `INIT_COMPLETE` | **OK** |
| WiFi scan | `scan fin count=24` / `25` / `22` | **OK** |
| WiFi assoc (×2) | `ADD_STA` `0x18` ok → `SCD_QUEUE_CFG tfd=…` → `timeout … id=0x1d` | **FAIL** |
| DHCP / SSH | ethernet `phystatus 0x84` DOWN; sin lease WiFi | **pendiente** |

Secuencia assoc (1.ª y 2.ª intento, idéntica):

```
PHY_CONTEXT ch3 ok → MAC 0x28 ok → ADD_STA 0x18 seq=0x0011 ok
→ SCD_QUEUE_CFG tfd=0x213d6000 bc=0x213da000 cb_size=1 n=16 sta=0
→ timeout cmd grp=1 id=0x1d slot=18; MVM parado
→ SCD_QUEUE_CFG mgmt falló → fallo AUTH+ASSOC
```

---

## Hallazgos

### WIFI-1. El FW cc-a0-77 no implementa `SCD_QUEUE_CFG` legacy `0x1d` (confirmado)

**Síntoma:** Tras ADD_STA el driver envía HCMD **grupo 1, id `0x1d`**, 24 B.
No llega ningún RX; timeout sin NACK.

**TLV `CMD_VERSIONS` en `iwlwifi-cc-a0-77.ucode`:** existe
`(group=5, cmd=0x17, ver=3)` para `SCD_QUEUE_CONFIG_CMD`. **No** hay entrada
`(group=1, cmd=0x1d)`.

**Linux 6.6.32:** cuando `queue_alloc_cmd_ver == 3`, envía
`iwl_scd_queue_cfg_cmd` (36 B) por **`DATA_PATH_GROUP` + `SCD_QUEUE_CONFIG_CMD`**
con `operation=ADD`, `sta_mask=BIT(sta_id)`, `tid=15`, `flags=0` — **no** el
`0x1d` de 24 B ([`queue/tx.c:1264–1276`](lxdde/linux/drivers/net/wireless/intel/iwlwifi/queue/tx.c)).

**soso (USB):** [`iwl_trans_txq_alloc_mgmt`](lxdde/ports/iwlwifi/iwl_trans.c)
sigue mandando LEGACY `SCD_QUEUE_CFG` `0x1d`. El log muestra `cb_size=1`
(coherente con el payload legacy) pero el FW no lo reconoce.

**Nota run9:** La hipótesis «TFD a ceros sin `invalid_tx_cmd`» era necesaria
pero **no suficiente**: en este arranque el log ya incluye direcciones DMA
(`tfd=0x213d6000 bc=0x213da000`) y el timeout persiste porque el HCMD es el
equivocado, no porque falte prefetch.

### WIFI-2. Misma entrada v3 en so-a0-89 (AX211)

El cambio a `SCD_QUEUE_CONFIG_CMD` v3 aplica también a AX211; el hostcheck
parsea ambos UCODES.

### Descartados (no bloquean assoc en este ciclo)

- Teclado 18c6: OK (regresión run2/run13 resuelta).
- GSP / `ask hola`: OK.
- G6 32 GiB / nombre GA104: ya en HEAD; este USB aún `familia=ga107`, techo 8192 MiB.
- Ethernet DOWN: sin cable.

---

## Orden de corrección (implementado en HEAD)

1. **`iwl_internal.h`:** `SCD_QUEUE_CONFIG_CMD 0x17`, `struct iwl_scd_queue_cfg_cmd` (36 B).
2. **`iwl_trans.c` `iwl_trans_txq_alloc_mgmt`:** `scd_ver = iwl_fw_cmd_ver(DATA_PATH, 0x17)`;
   si `ver==3` → HCMD grupo 5 / `0x17` / 36 B (`ADD`, `sta_mask`, `flags=0`);
   conservar `invalid_tx_cmd` + BC 640 B + TFD 16×256; legacy `0x1d` solo si `ver==0`.
3. **`assoc_abi_test` / `cdb_lmac_test`:** exigen grupo 5, id `0x17`, 36 B; fallan si
   aparece `0x1d` 24 B; `ADD_STA_KEY` buscado por `(LEGACY_GROUP, 0x17)` (colisión id con SCD v3).
4. **Este informe + `hw-matrix.json`** (`ax200-wifi` / `ga107-igpu`, logs 14-sep).

---

## Validación en placa (usuario)

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

Tras `wifi connect SSID PSK`: SOSOLOG debe mostrar **`SCD_QUEUE_CONFIG`** (grp=5 id=0x17)
sin `timeout … id=0x1d`; `TXQ mgmt qid≠0`; siguiente HCMD = **`SESSION_PROTECTION`**; luego AUTH.

---

## Qué no se ha hecho

- No reflasheado ni validado en placa en este ciclo.
- No 4-way, DHCP WiFi ni SSH.
- Ethernet sin cable. GB205/AX211 ausentes en este hwscan.
