# ROG: WiFi SCD + ask G6 VMM (14 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-14/` (ESP desmontada).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`dfec53f84-dirty`)** |
| Flush WiFi | **#101 @ ~1570 s uptime** (reval 14 sep) |
| Flush ask | **#101** (misma sesión; log 256 KiB lleno) |
| Hardware | `10de:249c` (GA104 RTX 3080 Laptop 16 GiB) + `8086:2723` (AX200) |
| Userspace | **`sosh —`** OK |
| GPU | `GSP_INIT_DONE`, pool VRAM OK, eager=true residente=true |
| WiFi assoc (flush #101) | LQ ok → `SCD_QUEUE_CONFIG v3 tid=15` timeout slot=19 |
| `ask hola` (flush #101) | Eager 291/3446 ms OK; prefetch USB ~23 min; `on_gpu=0` todas capas |

Árbol Linux WiFi: **torvalds/linux v6.6** (`mvm/sta.h`, `pcie/tx-gen2.c`).
G6/ask: diseño interno [`gsp_compute.c`](../lxdde/ports/nouveau/gsp_compute.c),
[`gsp_vmm.c`](../lxdde/ports/nouveau/gsp_vmm.c), [`PLAN-VRAM-16G-ROG.md`](PLAN-VRAM-16G-ROG.md).

---

## Tabla de etapas ask (flush #59 @ 3027479 ms)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Catálogo | `askd: catálogo+disco mistral-7b — 149 ms` | **OK** |
| Backend GPU | `askd: backend GPU (+0 ms)` | **OK** |
| VRAM plan | `modelo 4495 MiB, libre 16056 MiB, eager=true, residente=true` | **OK** |
| Subida eager | `283 ok`, `4427022336 B en 1384219 ms` (~23 min) | **FAIL** (lenta; 8 tensores) |
| VMM G6 | `VA 0x819d800000 es página grande; no cabe hoja de 4 KiB`, `sin sitio tablas (96)` | **FAIL** |
| Inferencia | `capa N/32 … on_gpu=0` (~80 ms/capa, CPU) | **FAIL** |
| Respuesta | `prefill token 4/11`; log 256 KiB lleno; **sin `generar rc=0`** | **FAIL** |

---

## Hallazgos ask (confirmados)

### ASK-1. Scratch x/y en VRAM choca con pesos 2 MiB (confirmado)

**Síntoma:** `on_gpu=0` en todas las capas; spam G6 en cada matvec.

**soso:** [`user/soso-gpu/src/lib.rs`](../user/soso-gpu/src/lib.rs) — con
`pesos_fijos`, `ensure_scratch(..., vram: true)` reservaba G6 con PTE 4 KiB
encima de pesos en páginas grandes.

**Kernel:** [`gsp_compute_matvec_resident`](../lxdde/ports/nouveau/gsp_compute.c) —
x/y en `G6_RES_VA` (sysmem, mapeado al init); el submit MATVF sólo pasa `w_va`.

**Fix implementado:** scratch x/y siempre GART (`vram: false`).

### ASK-2. Tablas VMM y banda 4 KiB (confirmado)

**Síntoma:** `sin sitio para más tablas (96)`; 8 tensores sin subir.

**Fix implementado:** `GSP_VMM_MAX_PT` 192; banda `G6_SMALL_VA_BASE` para
tensores &lt; 2 MiB separada del bump de pesos grandes.

### ASK-3. Subida eager sin prefetch USB (confirmado)

**Síntoma:** ~23 min leyendo shards desde USB vía `tensor_view`.

**Fix implementado:** `prefetch_shards_logged` antes del bucle eager; progreso
socket cada 8 tensores.

### ASK-4. UX silenciosa (confirmado)

**Fix implementado:** `probe_compute()` tras cargar sesión; aviso si GPU no
calcula; rate-limit `lx_printk` G6/VMM.

---

## Orden de corrección (ask) — implementado

1. Scratch GART en `soso-gpu` — **hecho**
2. Prefetch + progreso en `soso-llm/main.rs` — **hecho**
3. VMM 192 pt + banda pequeña en `gsp_buf.c` / `gsp_vmm.h` — **hecho**
4. Probe + rate-limit en `ask.rs` / `gsp_vmm.c` — **hecho**
5. Validación placa — **pendiente**

**Placa:** tras flash rootfs+kernel, `ask hola` debe mostrar prefetch, subida
&lt; ~5 min, `on_gpu=1` en capa 1, respuesta o aviso explícito de CPU lenta.

```bash
cargo xtask build
cargo xtask flash-usb-live /dev/sda --yes --only rootfs,kernel
```

---

## WiFi — revalidación placa (flush #59, 14 sep 2026)

Logs: `target/usb-diagnostic-2026-09-14-wifi/` (ESP `/dev/sda1`, desmontada).

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| ALIVE / init / scan | `UCODE_ALIVE_NTFY`, scan autoconnect OK | **OK** |
| `wifi connect Rutilo` | PSK derivada, BSS ch40 WPA2 | **OK** (scan) |
| PHY assoc | `PHY_CONTEXT ch40 band=0 action=1 ok` | **OK** |
| LQ_CMD | `LQ_CMD AP sta_id=0 (6 Mbps legacy)` | **OK** |
| SCD mgmt | `SCD_QUEUE_CONFIG … id=0x17 ver=3` → `timeout slot=22; MVM parado` | **FAIL** |
| AUTH / 4-way | `TXQ mgmt falló`; sin `rx AUTH` | **FAIL** |

Secuencia:

```
wifi connect Rutilo → LQ_CMD async ok
→ SCD_QUEUE_CONFIG v3 (tfd=0x213d7000 bc=0x213db000)
→ timeout grp=5 id=0x17 slot=22
→ wifi-wpa: fallo AUTH+ASSOC
```

### Hallazgo WIFI-6: tid=8 en SCD v3 es incorrecto (confirmado vs Linux v6.6)

**soso (regresión):** `IWL_MGMT_TID = IWL_MAX_TID_COUNT (8)` en payload SCD.

**Linux v6.6** `mvm/sta.c:852-853` — `iwl_mvm_tvqm_enable_txq`: si
`tid == IWL_MAX_TID_COUNT (8)` → **`tid = IWL_MGMT_TID (15)`** antes de
`iwl_trans_txq_alloc` / SCD v3.

**Fix:** `IWL_MGMT_TID=15` en [`iwl_internal.h`](../lxdde/ports/iwlwifi/iwl_internal.h);
log SCD incluye `tid=` para validar en placa.

**Placa:** SOSOLOG muestra `tid=15` en SCD y `TXQ mgmt qid=N` sin timeout.

Hostcheck: `./scripts/l6-iwl-fw-hostcheck.sh` OK tras corrección tid.

---

## Matriz (manual, sin `--boot-ok`)

| id | ok | fail |
|---|---|---|
| `ga107-igpu` | gsp_rpc, vram_pool, ce_readback | carga_real, compute_cpu_gpu (flush#59) |
| `ax200-wifi` | alive, init, mvm, scan | assoc_wpa2 (flush#40 SCD tid=15) |

---

## Revalidación flush #101 (14 sep 2026, post-fixes previos)

Lo que **ya va** en esta lectura ESP:

- GSP: `GSP_INIT_DONE`, pool VRAM, CE GO, familia GA104, ventana VA 32 GiB.
- Pesos: prefetch 291/291 + subida eager **291 tensores / 3446 ms / dma=291 bounce=0**.
- WiFi: `UCODE_ALIVE_NTFY`, scan 21 BSS, `tid=15` en wire SCD.

Lo que **sigue roto** (flush #101):

```
wifi connect Rutilo → LQ_CMD async → SCD v3 tid=15 → timeout slot=19; MVM parado
ask hola → GPU no calcula (VMM) → on_gpu=0; sin línea QMD ni MATVF
prefetch USB ~23 min (1383 s) — ancho de banda BOT, no VMM
```

### Fix WIFI-7: LQ async + SCD no bloqueado (revert sync)

**soso (regresión flush #28):** LQ_CMD **síncrono** hace timeout
(`grp=1 id=0x4e seq=0x0012 slot=18; MVM parado`) y **impide** llegar a SCD.

**Linux v6.6** `mvm/utils.c:253`: LQ es **CMD_ASYNC**; no se espera ACK antes de
`iwl_trans_txq_alloc`. AUTH lleva CMD_RATE sin rate scale.

**Fix:** LQ_CMD **async** otra vez; si no encola, **igual** SCD; log RX sin emparejar
durante timeout HCMD (SCD pendiente de validar en placa).

### Fix GPU-8: QMD Ampere sin REQUIRE_SCHEDULING_PCAS

**soso:** `gsp_compute_fill_qmd_v02_grid` no ponía `QMDV02_REQUIRE_SCHEDULING_PCAS`;
`stage_sass_kernel` exigía scratch antes de comprobar `k->staged`; MATVF fallaba
sin printk; `ask` decía «VMM» genérico.

**Fix:** bit PCAS en QMD v2; `staged` antes de scratch; printk único MATVF;
`ask` muestra `last_fail` real.

Host: `./scripts/l6-iwl-fw-hostcheck.sh`, `./scripts/l6-g3-gsp-hostcheck.sh`.

**Placa:** `cargo xtask flash-usb-live /dev/sda --yes --only kernel` →
`wifi connect` sin timeout SCD; `ask hola` con `on_gpu=1` capa 1 o línea kernel.

---

## Revalidación flush #28 (14 sep 2026, kernel dfec53f84-dirty)

Lectura ESP `/media/jmdiez/KERNEL` → `target/usb-diagnostic-2026-09-14/SOSOLOG.TXT`.

| Campo | Valor |
|---|---|
| Flush | **#28 @ 116 s uptime** |
| Kernel | **0.2.2 (`dfec53f84-dirty`)** — incluye LQ sync + QMD PCAS |
| WiFi | `wifi connect Rutilo puentelasierra` → ADD_STA ok → **LQ timeout** → MVM parado |
| GPU | **sin `ask hola`** en el log (no validado en esta sesión) |

Secuencia WiFi:

```
ADD_STA seq=0x0011 ok
→ timeout cmd grp=1 id=0x4e (LQ_CMD sync) slot=18; MVM parado
→ LQ_CMD falló; SCD igual   ← no hay línea SCD_QUEUE_CONFIG ni TXQ mgmt
```

**Conclusión:** el fix WIFI-7 (LQ sync) empeora el caso; revertido a async en código.
SCD v3 sigue pendiente de revalidar (flush #101 tenía LQ ok + SCD timeout).

GPU: QMD PCAS + MATVF printk en kernel; falta `ask hola` tras reflashear.

---

## Qué no se ha hecho

- Reflashear USB con LQ async revertido y repetir `wifi connect` + `ask hola`.
- No reflasheado USB en esta ejecución del agente.
