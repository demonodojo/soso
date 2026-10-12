# Auditoría GB205 — port vs referencias open (2026-07-29)

Referencias locales: `lxdde/reference/README.md`.

Contraste de [`lxdde/ports/nouveau/`](../lxdde/ports/nouveau/) contra:

- **OGKM 570.144** — `open-gpu-kernel-modules-570.144/` (misma versión que firmware)
- **nouveau master** — `linux-master-nouveau/drivers/gpu/drm/nouveau/` (gb202, r570)

## Resumen ejecutivo

| Área | Estado port | Referencia | Veredicto |
|------|-------------|------------|-----------|
| GFW boot (`0x118128`/`0x118234`) | Ya no bloquea en Blackwell | Scratch GC6 **Turing**; gb20x usa FSP | **Falso positivo** — resuelto |
| FSP boot complete | `fsp_ready_to_send()` | `0xad00bc == 0xff` (gb202) | **OK** |
| VFN usermode / doorbell MMIO | `0xbb0000` + `0x90` | `ga100_vfn`: priv `0xb80000`, user `+0x030000` | **OK** (herencia ga100) |
| Formato token doorbell | Token de RPC tal cual | gb202: `BIT(30) \| (runlist_id << 16) \| chid` | **Revisar** si RPC devuelve bit 30 |
| Kick doorbell bajo GSP-RM | `gsp_mmio_wr32(0xbb0090, token)` | Host: `GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_DOORBELL)`; **GSP: `kfifoUpdateInternalDoorbellForUsermode_*`** | **Desviación probable** — causa de `GPGet=0` |
| Volcado runlist PRI | `chan_dump_hw` lee `runl+0x100` | `0xbadfxxxx` = patrón PRI dummy (acceso denegado) | **No es FIFO sin init** — abandonar lectura BAR0 |
| PTOP / runlist base | `engineData[11]` | PTOP HW: GR0/CE0 `0xd00000` | **Dirección OK** |

## 1. GFW boot (Turing heredado)

**Port (antes):** `gsp_mmio_gfw_wait()` leía `0x118128` bit0 y `0x118234 & 0xff == 0xff` (`tu102_devinit_wait`).

**Referencia:** En gb202 published headers no hay equivalente usable en esas direcciones; en Turing `0x118128` es `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK`, no estado de boot.

**OGKM:** `kfspWaitForSecureBoot_*` usa `NV_THERM_I2CS_SCRATCH` = `0xff`; en gb20x la dirección es **`0x00ad00bc`** (`dev_therm.h` gb202), no `0x000200bc` (gh100).

**Acción (hecha):** No llamar `gsp_mmio_gfw_wait` en Blackwell; criterio en `g3-check` = `FSP secure boot=0x000000ff`.

## 2. Aperture usermode y doorbell

### Offsets VFN

**nouveau master** [`nvkm/subdev/vfn/ga100.c`](../lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/subdev/vfn/ga100.c):

```c
.user = { 0x030000, 0x010000, ... };
return nvkm_vfn_new_(..., 0xb80000, ...);  // user = 0xbb0000
```

**Port:** `NV_VFN_USERMODE_BASE 0xbb0000`, `NV_VFN_DOORBELL +0x90` — coincide con `tu102_chan_start` → `device->vfn->addr.user + 0x0090`.

**Sonda TIME:** `chan_probe_usermode()` compara usermode+0x80/0x84 con `NV04_PTIMER_TIME_0` (0x9400). Alineado con comentarios en `clc361` / Volta+.

### Formato del token (Blackwell)

Evolución en nouveau [`engine/fifo/`](../lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/engine/fifo/):

| Chip | `doorbell_handle` |
|------|-------------------|
| tu102 | `(runl->id << 16) \| chan->id` |
| ga100 | `(runl->doorbell << 16) \| chan->id` |
| **gb202** | **`BIT(30) \| (runl->id << 16) \| chan->id`** |

**OGKM gb202** [`kernel_fifo_gb202.c`](../lxdde/reference/open-gpu-kernel-modules-570.144/src/nvidia/src/kernel/gpu/fifo/arch/blackwell/kernel_fifo_gb202.c) (host RM): campos `_VIRTUAL_FUNCTION_DOORBELL` con `RUNLIST_DOORBELL_ENABLE`.

**Port:** Comentario asume `(runl->doorbell << 16) \| chid` (Ampere). En HW reciente el token RPC fue `0` (runlist 0, chid 0) — válido como chid pero **puede faltar bit 30** en gb202.

**Acción recomendada:** Tras `GET_WORK_SUBMIT_TOKEN`, comparar con `0x40000000 | (runlist_id << 16) | chid` si RM no incluye bit 30; registrar en log la máscara `token & 0x40000000`.

### Kick doorbell bajo GSP-RM (incógnita principal)

**Port:** Tras publicar `GPPut`, escribe directamente `gsp_mmio_wr32(NV_VFN_DOORBELL, token)` — copia de **host** `tu102` / `kfifoUpdateUsermodeDoorbell_TU102` → `GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_DOORBELL, ...)`.

**OGKM** [`kernel_fifo_ga100.c`](../lxdde/reference/open-gpu-kernel-modules-570.144/src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c):

```c
if (!RMCFG_FEATURE_PLATFORM_GSP)
    return kfifoUpdateUsermodeDoorbell_TU102(...);
else
    return kfifoUpdateInternalDoorbellForUsermode_HAL(...);  // GB202 tiene variante propia
```

En la ruta **GSP** (nuestro caso: GSP-RM en RISC-V), el driver **no** escribe el doorbell de usermode como en host; usa doorbell **interno** (`kfifoUpdateInternalDoorbellForUsermode_GB202`, declarado en `g_kernel_fifo_nvoc.h`).

**nouveau host** además en `ga100_chan_start` escribe **`runl->addr + 0x0090`** con `(gfid << 16) | chan->id` (INTERNAL_DOORBELL) y en `ga100_runl_init` habilita doorbell en **`runl + 0x300`** bit 31.

**Port:** Solo BIND + SCHEDULE RPC; no replica init de runlist ni internal doorbell.

**Acción recomendada (prioridad 1 para G4e):**

1. Buscar en OGKM/r570 si existe control RM equivalente a “poke doorbell” en ruta GSP (p.ej. notificación tras submit, o RPC que llame `kfifoUpdateInternalDoorbellForUsermode`).
2. Si no hay RPC, evaluar escritura a **`runlist_base + 0x0090`** (internal) además o en lugar de `0xbb0090`, tras confirmar runlist id con PTOP.
3. No gastar más ciclos interpretando `chan_dump_hw` en runlist PRI.

## 3. PRI `0xbadfxxxx` — privilegio, no “FIFO vacío”

**Port:** `gsp_mmio_pri_error()` trata `0xbadf0000` como error PRI; `chan_dump_hw` leía `runl+0x100` → `0xbadf5040`.

**Referencia OGKM** [`gb100/pri_nv_xal_ep.h`](../lxdde/reference/open-gpu-kernel-modules-570.144/src/common/inc/swref/published/blackwell/gb100/pri_nv_xal_ep.h):

```c
#define NV_XAL_EP_SCPM_PRI_DUMMY_DATA_PATTERN_INIT  0xbadf0200  /* RWI-V */
```

Lectura PRI sin nivel suficiente devuelve **patrón dummy `0xbadf…`**, no datos del bloque. Bajo VFIO el guest suele estar en PL0; muchos bloques FIFO/runlist exigen PL ≥ 1 en RM.

**PTOP HW:** Runlist bases correctas (`0xd00000`); el fallo es **acceso**, no dirección.

**Acción recomendada:** Estado del canal vía RM (`NV2080_CTRL_CMD_FIFO_*`, status GPFIFO) en lugar de BAR0; mantener `chan_dump_hw` solo como aviso “PRI no legible”.

## 4. Secuencia canal (BIND / SCHEDULE / token)

**Port:** `chan_bind_engine` → `chan_schedule` → `GET_WORK_SUBMIT_TOKEN` → submit con doorbell. Orden alineado con controles `NVA06F_*` / `NVC36F_*` en `nvrm_r570.h`.

**Desviaciones conocidas (ya corregidas en port):**

- Clases: `BLACKWELL_CHANNEL_GPFIFO_B` / `BLACKWELL_DMA_COPY_B` vía `GET_CLASSLIST_V2`.
- USERD index en flags de alloc (`chid % 8`, `chid / 8`).
- `SET_OBJECT` como método de 2 palabras, no inmediato.

**Pendiente:** Contexto GR promocionado (`gsp_grctx.c`) antes de compute; CE no lo exige pero GR0 `NO_MEMORY` sí bloquea G4f.

## 5. Offsets heredados — tabla rápida

| Offset / registro | Port | Origen referencia | gb205 |
|-------------------|------|-------------------|-------|
| FSP boot | `0xad00bc` | OGKM gb202 `dev_therm` | Verificado HW |
| GFW GC6 | `0x118128/234` | Turing only | **No usar** |
| PTIMER | `0x9400` | Común | OK |
| VFN user | `0xbb0000` | ga100_vfn | OK |
| Doorbell user | `0xbb0090` | tu102+ | OK host; **GSP usa internal** |
| Runlist internal DB | `runl+0x90` | ga100_chan_start | **Port no escribe** |
| Runlist DB enable | `runl+0x300` bit 31 | ga100_runl_init | **Port no escribe** |

## 6. Correlación con ciclo HW (2026-07-29, `target/g1-vfio-serial.log`)

`cargo xtask g3-check`: **21 OK, 0 FAIL**. Criterios relevantes:

| Criterio | Resultado | Log |
|----------|-----------|-----|
| G3b hw boot | GO | `GSP booted (hw, GSP-FMC vía FSP)` |
| FSP boot complete | GO | `FSP secure boot=0x000000ff` |
| GFW NOCAT | NOTA (ruido) | ASSERT `GFW_BOOT_PROGRESS` — scratch Turing |
| PTOP | GO | CE0 runlist `0xd00000` = tabla FIFO |
| usermode aperture | GO | reloj avanza en `0xbb0000` |
| G4e readback CE | **GO** (tarde del 2026-07-29) | `CE readback verificado (G4e GO)` |

Datos que confirmaron la auditoría en la mañana (ya superados):

- `doorbell token=0x00000000 (runlist=0 chid=0)` — **sin bit 30** (`0x40000000`) que exige nouveau gb202.
- `INTR_0=badf5040` al leer runlist — patrón PRI dummy, no estado FIFO.
- `dbcfg=00000002 → doorbell=0` en una lectura y `dbcfg=00020004 → doorbell=2` en otra — coherente con acceso PRI parcial, no con token RPC.

Conclusión: el bloqueo G4e **no** es GFW ni aperture usermode; encaja con **kick doorbell incorrecto en ruta GSP** (sección 2).

### Resultado del ciclo con el kick corregido (2026-07-29, 09:12)

El bit 30 se implementó (`gsp_chip.c`, `gsp_chan_doorbell_kick`) y el ciclo lo confirma
en el log: `doorbell RPC=0x00000000 kick=0x40000000 bit30=1` (CE) y `kick=0x40000001`
(GR0) — exactamente el valor de `gb202_chan_doorbell_handle`. **Y `GPGet` sigue a 0.**
La hipótesis queda contrastada en silicio y descartada como causa única; el kick con
bit 30 se queda (es lo que escribe upstream y sin él el timbre es DISABLE por
definición), pero el bloqueo está en otro eslabón.

Lo que este ciclo acota:

- Todo el kick está ya verificado contra gb202: dirección (`0xbb0090`, aperture vivo
  por la sonda TIME), valor (bit 30 + runlist + chid) y orden (GPPut publicado antes).
- El `dbcfg` leído del bloque de runlist volvió a cambiar entre volcados
  (`0x0` → `0x2`; ayer `0x00020004`) con `INTR_0=badf5040`: ese bloque PRI no es
  legible desde PL0 y sus lecturas no son datos (sección 3).
- Queda una desviación conocida de upstream en la cadena doorbell→fetch: el **USERD
  en sysmem** (nouveau lo pone en VRAM, `userdMem.addressSpace=2`; el port lo declara
  `SYSMEM` porque sin BAR1 la CPU no tiene ventana a VRAM). RM aceptó el alloc, pero
  que ESCHED sondee un USERD en sysmem en gb202 no está verificado en ninguna de las
  dos referencias. Candidato a investigar junto con el estado del canal en CHRAM
  (`NV_CHRAM_CHANNEL_*` de `gb202/dev_runlist.h`, en curso en la otra sesión), que es
  el instrumento que dirá si el ESCHED llegó a ver el timbre.

### Resultado del ciclo con PRAMIN + USERD en VRAM (2026-07-29, ~10:45)

Ciclo tras arranque en frío (los dos anteriores fallaron en el bring-up del FMC por
estado residual de la tarjeta: `mbox0=0x0001293f` y luego falcon en `badf4100`; el
apagado completo lo limpió). Las tres capas del plan «Ventana BAR0 a VRAM» hablaron:

- **(a)** `PRAMIN ventana BAR0 viva (0x700000 + offset)` — la ventana `0x10fd40`
  NO está bloqueada por SCPM. La CPU lee/escribe VRAM con el GSP vivo.
- **(b)** El RAMFC del canal CE (inst 0x401000) se leyó entero, con datos reales de
  RM. Ojo: el decodificador con offsets gv100 (+0x008 USERD, +0x048 GPFIFO) NO vale
  en gb202 — imprime `NO_COINCIDE` pero los valores están, en otros sitios:
  USERD como página (`0x400` = 0x400000 nuestro ✓) en +0x020, base del GPFIFO
  (`0x8020000000` ✓) en +0x090.
- **La bomba**: el contexto salvado contiene VAs derivadas del pushbuffer
  (+0x080 `0x8020002020`, +0x088 `0x8020002068` = base+0x20/base+0x68) y cabeceras
  de método cacheadas (`0x20010101` = OFFSET_IN_LOWER, `0x20010104` + dato `0x1000`
  = PITCH_IN). Esos valores solo puede calcularlos el PBDMA leyendo nuestra GP
  entry. **El doorbell funciona, ESCHED planificó el canal, el PBDMA leyó el GPFIFO
  y el pushbuffer a través del vaspace.**
- Y el GSP dijo el resto sin que se lo preguntáramos: NOCAT con
  `FIFO GEN EXCP FAULTING_APP PBDMA_HANG_DURING_HTE` (+ `GR_STATUS`) y dos
  `RC_TRIGGERED` (fn=0x1004, 6624 B). El PBDMA se cuelga **entregando los métodos
  al motor** (HTE = host→engine) y RM hace RC del canal. `GPGet=0` explicado: la
  entrada nunca se completó.

El bloqueo de G4e ya no es el timbre ni el fetch: es que **el motor no consume los
métodos**. Sospechosos, por orden: el emparejamiento GRCE (COPY0 comparte la runlist
0xd00000 con GR según PTOP; las entradas de runlist de gb202 tienen
`NV_RAMRL_ENTRY_CHAN_RUNQUEUE_SELECTOR_RUNQUEUE1` — el ÚNICO define que NVIDIA
publica en `gb202/dev_ram.h`), el `SET_OBJECT`/subchannel del CE, y el contenido
del RC_TRIGGERED (6624 B sin parsear que probablemente digan el motivo exacto).

## 7. Resolución (tarde del 2026-07-29) — G4e/G4f/G5 GO

El bloqueo no era un solo eslabón. Cerrado en silicio:

| Causa | Fix |
|-------|-----|
| Enlace PCIe Gen5 inestable en reset FMC | Cap Gen3 en root port (`setpci CAP_EXP+0x30.w=3:f` + retrain) |
| `IMMD_DATA_METHOD` mal (dato como dword suelto) | `cp_pb_immd`: dato en bits 28:16; PCAS = 24 B |
| USERD `GPGet` siempre 0 en Blackwell | Esperado (sin writeback); `gsp_chan_ack_progress` tras sem CE/QMD |
| `GPPut=64` con Get USERD=0 → PBDMA hang | GPFIFO 4096 entradas × 8 B + VAs sin solapar PB |

Ciclo verde: `CE readback verificado`, ~97 `matvec en GPU OK` con
`soso-llm run tiny --max 4`, apagado `unload=ok halt=ok`. Detalle operativo en
skill `soso-gpu`.

Si un ciclo acaba con «GPU fuera del bus», NO relanzar: apagado en frío del host
(el reset de vfio sobre un enlace muerto colgó la máquina el 2026-07-29 09:50).

## Referencias citadas (paths locales)

- OGKM FIFO gb202: `lxdde/reference/open-gpu-kernel-modules-570.144/src/nvidia/src/kernel/gpu/fifo/arch/blackwell/kernel_fifo_gb202.c`
- OGKM doorbell GSP: `.../kernel_fifo_ga100.c` (`kfifoUpdateUsermodeDoorbell_GA100`)
- OGKM PRI dummy: `.../published/blackwell/gb100/pri_nv_xal_ep.h`
- nouveau gb202 doorbell: `lxdde/reference/linux-master-nouveau/drivers/gpu/drm/nouveau/nvkm/engine/fifo/gb202.c`
- nouveau ga100 runlist init: `.../engine/fifo/ga100.c`
