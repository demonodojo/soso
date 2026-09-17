# ROG GA104 + AX200 — arranque validado (17 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
[`target/usb-diagnostic-2026-09-17-run2/`](../target/usb-diagnostic-2026-09-17-run2/).
Credenciales WiFi en ESP (`SOSOWIFI.TXT`); **no** se incluye PSK en este informe.

| Campo | Valor (flush **#31**, último arranque en el log) |
|---|---|
| Kernel en placa (SOSOLOG) | **0.2.2 (`a8f6be44f-dirty`)** — commit `testing ssh` |
| Flush / uptime | **#31 @ 4 616 702 ms** (~77 min); ~52 KiB / 256 KiB |
| Hardware | `10de:249c` (GA104 Ampere 16 GiB) + `8086:2723` (AX200 gen2) + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** pid=2; teclado; **`halt`** |
| WiFi | ALIVE + MVM + scan **21/26 BSS**; **4-way + DHCP LxWifi** `192.168.68.132/24` |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; CE readback; compute sm_86 listo (sin `ask` en log) |
| SSH | sesión pid=3, cierre, sesión pid=4 (reconexión) |
| Ethernet | rtl8169 enlace DOWN (sin cable); lease WiFi, no confundir con `net: dhcp` previo en rtl8169 |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** — iwlwifi `pcie/rx.c`, nouveau GSP ga102 |
| [`lxdde/reference/linux-master-nouveau/`](../lxdde/reference/linux-master-nouveau/) | referencia BAR1 / RM (no parchear PDB instancia) |

Hostchecks (esta sesión, host):

- `./scripts/l6-iwl-fw-hostcheck.sh` — **OK**
- `./scripts/l6-g3-gsp-hostcheck.sh` — **OK**

No acreditan DMA WiFi ni matvec/`ask` en silicio.

Matriz A8 actualizada: [`hw-matrix.json`](hw-matrix.json) — entradas `ga107-igpu`, `ax200-wifi`.
**Racha:** `arranques_consecutivos_ok=1` (sin `record-boot`; el arranque flush #39 con sosh→kshell sigue en historial con `userspace=false`).

---

## Tabla de etapas (flush #31)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / shim | `BOOTMARK`: UEFI → `bootsoso.efi` | **OK** |
| Boot / sosh | `soso 0.2.2 (a8f6be44f-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#31** @ ~77 min | OK |
| USB live | GPT `backend=Usb`; mass storage 048d:1234 | **OK** |
| xHCI teclado | slot=3 `0b05:18c6`: timeout EP0 intento 1/4 → reintento OK | **OK** (ruido) |
| GPU GSP | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado (G4e GO)` | **OK** |
| GPU compute | `compute listo` sm_86; sin `matvec en GPU` / `ask` en sesión | **pendiente** |
| GPU halt | `halt`; `GSP se deja (placa; no hay reset VFIO)` — sin `dma=off` | parcial |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`; familia 22000 gen2 | **OK** |
| WiFi init | `INIT_COMPLETE_NOTIF`; `up mínimo listo` | **OK** |
| Scan | `SCAN_COMPLETE count=21` / `26` (autoconnect) | **OK** |
| AUTH / ASSOC | `mlme_auth_ok`; `asociado a 'Rutilo' aid=3` | **OK** |
| 4-way | `wifi-wpa: 4-way completado, enlace autorizado` | **OK** |
| DHCP LxWifi | `net: backend lx-wifi`; `net: dhcp 192.168.68.132/24 gw 192.168.68.1` | **OK** |
| SSH | `ssh: sesión abierta` pid=3/4; cierre entre medias | **OK** |
| Apagado | `$ halt` → `apagando…` | **OK** |

---

## Hallazgos

### Descartados como bloqueo (confirmado vs Linux)

**1. SCD-wait / RX sin emparejar durante SCD_QUEUE_CONFIG — descartado**

Síntoma: `iwl_rx: [SCD-wait] grp=5 id=0xf7 seq=0xc00a … (esperando seq=0x0015)` y
`sin emparejar (esperando grp=5 id=0x17 seq=0x0015)`.

- soso: [`lxdde/ports/iwlwifi/iwl_trans.c`](../lxdde/ports/iwlwifi/iwl_trans.c) (`log_rx`, `handle_gen2_rx` ~591–948).
- Linux: reclama HCMD solo si `!(pkt->hdr.sequence & SEQ_RX_FRAME)` — [`pcie/rx.c:1363-1401`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/rx.c).

Compatible con notificaciones de cola distintas mientras el SCD sincrónico espera; a continuación `TXQ mgmt` y assoc completan con éxito.

**2. BAR1 PDB instancia ≠ bar1PdeBase — descartado**

Síntoma: `PDB=0x31b233f89e94b000` vs `bar1PdeBase 0x3f3c2a000`; tablas PD3 inválidas en CPU.

- soso: [`gsp_bar1.c`](../lxdde/ports/nouveau/gsp_bar1.c), [`gsp_bringup.c:833-853`](../lxdde/ports/nouveau/gsp_bringup.c) — no parchear; mensaje «Linux envuelve rm_bar1_pdb».

CE selftest y GSP RPC siguen en verde en el mismo arranque.

**3. xHCI SET_CONFIGURATION timeout teclado secundario — descartado**

Síntoma: `transfer event timeout` slot=3 dci=1; `SET_CONFIGURATION failed` intento 1/4; reintento puerto 4 OK.

- soso: [`crates/xhci-nostd/src/driver.rs`](../crates/xhci-nostd/src/driver.rs) — timeout 5 s, `PORT_INIT_TRIES`.

**4. rtl8169 DOWN / iGPU AMD sin driver — fuera de alcance**

Sin cable ethernet; `1002:1638` sin port nouveau en live-usb.

---

## Flush #44 — `ask` + `tiny` (17 sep 2026, tarde)

Copias: [`target/usb-diagnostic-2026-09-17-run3/`](../target/usb-diagnostic-2026-09-17-run3/).

| Campo | Valor |
|---|---|
| Kernel | **0.2.2 (`a8f6be44f-dirty`)** (mismo que flush #31) |
| Flush | **#44 @ 344 211 ms** (~5,7 min) |
| WiFi / red | Igual patrón: ALIVE, 4-way Rutilo, DHCP `192.168.68.132` |
| Userspace | `sosh —` → intento modelo erróneo → `ask :modelo tiny` → `ask hola` → `halt` |
| Compute | **FAIL**: 0 matvec GPU; inferencia en CPU tras fallo de subida |

Secuencia relevante:

```
$ ask :modelo tiny --max 4    ← askd interpreta el modelo literal «tiny --max 4»
ask: no hay ningún modelo «tiny --max 4» en /models
$ ask :modelo tiny           ← OK
askd: subida GPU 1/34 (L00.attn_norm) … CE semáforo timeout 2000 ms
NOCAT: PBDMA_HANG_DURING_HTE / GEN EXCP
rc: FIFO_ERROR_MMU_ERR_FLT PTE @0x8031001000
soso-llm: offload GPU desactivado — subida de pesos
soso-llm: 0 matvec … último on_gpu=0  (132 tokens en CPU, 119 tok/s engañoso)
```

**Confirmado:** el selftest CE de arranque (4 KiB ida/vuelta) pasa, pero la **primera subida G6** del tensor `L00.attn_norm` cuelga el canal CE. El dump muestra `RAMFC USERD inst=0x0 enviado=0x4401000 NO_COINCIDE` y fault MMU en PTE.

**Descartado:** éxito de `compute_cpu_gpu` — el parser marcó `ok` por la línea `rendimiento GPU — 119.46 tok/s` con la subcadena `matvec`; en realidad **`0 matvec / 132 tokens`**.

---

## Orden de corrección

1. **CE G6 — footprint boa0b5 con origen +64 (fix aplicado 2026-09-17)**  
   Causa: primer tensor `L00.attn_norm` (512 B, puntero +64 del shard) iba por DMA con **1** PTE; el CE con encoding boa0b5 toca **4096 B** desde `G6_SRC_VA+64` → fault PTE en `G6_SRC+0x1000`, CE stuck, rebote inútil.  
   Fix: `gsp_ce_io_bytes()` + conteo de páginas en [`gsp_buf_upload_dma`](../lxdde/ports/nouveau/gsp_buf.c) y [`kernel/src/drivers/gpu.rs`](../kernel/src/drivers/gpu.rs) (`subir_por_dma`). Hostcheck: `+64 512 B` exige 2 páginas.  
   Placa: recompilar kernel/USB, `ask :modelo tiny` + `ask hola` → subida 1/34 OK, `matvec` GPU > 0.

2. **Uso de `ask` en placa (operativa, no código)**  
   Dos líneas: `ask :modelo tiny` y luego `ask :max 4` (askd); o `ask-modelo tiny` + editar `max=` en `/etc/llm.conf`. No escribir `ask :modelo tiny --max 4` como un solo nombre.

3. **Parser matriz — falso positivo tok/s**  
   [`xtask/src/hw_matrix.rs`](../xtask/src/hw_matrix.rs) `parse_gpu_stages`: exigir `matvec` con contador > 0 o `matvec en GPU OK`, no basta `rendimiento GPU … tok/s` con `0 matvec`.

4. **Registro**  
   Matriz `ga107-igpu` / `ax200-wifi`: racha **2/3**, `compute_cpu_gpu=fail`, logs run3. Sin `record-boot`.

---

## Qué no se ha hecho

- No se ha parcheado CE/G6 ni el parser de matriz.
- No se ha flasheado el USB.
- VFIO / `dma=off` en halt live: no aplica.
