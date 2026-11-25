---
name: soso-comp
description: >-
  soso GPU compositor (Sal `soso-comp`) — escena BIND/GEOM/DMAP/COMMIT, GSCEN/GFRAM,
  TCP :7432, tests Sal y crate soso-wm-proto. Usar al tocar `comp_*.sal`, present
  parcial, clip opaco, scanout con WM, o integración F8. WM clásico SHM: skill soso-wm;
  driver NVDisplay/GOP: skill soso-gpu.
---

# soso — Compositor GPU (Sal)

Daemon **`soso-comp`** escucha en **`127.0.0.1:7432`**. El WM (`soso-wm`) le envía la
escena; el compositor compone en RT (VRAM si `gfx3d`, si no SHM + `fb_present_rect`).

En modo GPU el WM usa superficie **`wm_surf_fondo()` id 19** (degradado pantalla completa,
z=0), barra z=5; menú/barra se pre-mezclan contra el fondo (sin alpha en GSCEN).

Protocolo detallado: [`references/gscen-protocol.md`](references/gscen-protocol.md).

Planes: compositor en [`docs/PLAN-NVDISPLAY.md`](../../docs/PLAN-NVDISPLAY.md) (F8),
escritorio en [`docs/PLAN-ESCRITORIO.md`](../../docs/PLAN-ESCRITORIO.md).

## Mapa de fuentes

| Módulo | Rol |
|--------|-----|
| `soso-comp.sal` | `main`, listener 7432, SHM compositor, `comp_servir_wm` |
| `comp_proto.sal` | Ops COMP_* y encoders 32 B |
| `comp_escena.sal` | Tabla superficies, commit, clip, armado `GSCEN` |
| `comp_gpu.sal` | Init VRAM, `GFRAM`, present rect |
| `comp_clip.sal` | Intersección / opacidad z-order |
| `comp_scanout_policy.sal` | RT w/h/pitch vs head NVDisplay |
| `comp_*_layout.sal` | Tamaños binarios GSCEN/GFRAM/GPRES/GNVD |
| `crates/soso-wm-proto/` | Layout Rust + tests host |
| `kernel/src/drivers/gpu.rs` | Handlers `GSCEN`, `GFRAM`, `GUPRC`, router present |

## Flujo de trabajo del agente

1. Leer `comp_escena.sal` y el handler `GSCEN` en kernel antes de cambiar el wire format.
2. Cualquier cambio de layout → actualizar **Sal layout + Rust crate + test Sal** en
   `tests/sal/escritorio/comp_*.sal`.
3. Recorte/opacidad → `comp_gscen_clip.sal` / `comp_escena_commit` deben seguir verdes.
4. Scanout / tamaño RT → `comp_scanout_policy.sal` y `gpu_scanout_effective` en `soso.sal`.
5. Rebuild: `cargo xtask sal-build` (instala `soso-comp` en rootfs).

## Comandos GPU (`SYS_GPU_SUBMIT`)

| Tag | Uso en compositor |
|-----|-------------------|
| `GUPRC` | SHM → textura VRAM (rect) |
| `GSCEN` | Quads texturizados + present dañado (hot path) |
| `GFRAM` | Blit CPU legacy hacia RT / shadow |
| `GPRIM` | Escena de primitivas 2D (GSCEN v2): sólido, degradado, rrect, anillo, rejilla de texto. Kernel SASS `gprim` por el canal de cómputo; layout en `soso_wm_proto::gprim` / `comp_gprim_layout.sal`; ≤32 prims, cabecera 48 B + prims + PSF + celdas |
| `GCLR3` | Clear RT (flag commit) |
| `GPRES` / `GNVD` | Present explícito (demo; WM usa router vía `GSCEN`) |

**PRIMS (op 37, WM→comp, :7432):** mensaje de 32 B (`len`@4, rect de present @8..24, flags@24) seguido de `len` bytes con un comando `GPRIM`. `soso-comp` rellena RT/dimensiones/pitch, lo lanza y contesta `PRIMS_ACK` (op 38, u32@4: 1 pintado, 0 = pinta tú en CPU); el primer rechazo del kernel se recuerda (`comp_st_gprim`). **El WM aún no lo emite** (el cromo se pinta en CPU). Prueba en la placa: cliente `gprim-prueba` y `docs/GPRIM-PRUEBA-HW.md`. En Sal el condicional es `elsif`, no `elif`.

Longitudes y offsets: módulos `comp_*_layout.sal` y submódulos en `soso-wm-proto`.

## Pruebas

```bash
python3 tests/sal/escritorio/probar_host.py comp_gscen.sal comp_gscen_clip.sal comp_scanout_policy.sal
cargo test -p soso-wm-proto
cargo xtask check
./scripts/l6-scanout-regression.sh   # política gop/nvdisp (F8)
```

Integración guest con GPU real: VFIO + `gfx3d` (skill **soso-gpu**); QEMU suele quedarse en GOP.

**Escena en CPU (`soso-comp --escena-cpu`)**: sin GPU de verdad, contesta `gpu_ok=1` al WM
y ejecuta el mismo `GSCEN` en CPU (`comp_gscen_cpu`: borrado opcional, quads opacos en orden
recortados a la pantalla, `fb_present_rect`). `comp_st_cpu()` lo marca. Cada superficie
tiene una «textura» propia (`bytes_new`, capacidad en `comp_surf_tcap`, se reutiliza si
cabe) que sólo se actualiza con el daño subido (`DMAP` + `COMMIT`), igual que la VRAM: así
un `DMAP` olvidado en el WM se ve también en QEMU. Lo lanza `soso-wm --escena-cpu`; las
pruebas, con `scripts/wm-probe.py --wm-args --escena-cpu`.

## Convenciones

- **Daño mínimo:** COMMIT lleva rect; GSCEN solo incluye quads visibles tras clip.
- **Opaco:** barra/menú/ventanas sólidas usan `GEOM_OPAQUE`; cursor no. Un quad de una
  superficie no opaca lleva el bit `8+i` en los flags de `GSCEN`
  (`comp_gscen_flag_blend`, `gscen::flag_blend`) y se **mezcla con alfa** (byte 3,
  sin premultiplicar). El CE sólo copia, así que el kernel baja a CPU el tramo de
  páginas del RT y de la textura, mezcla (`gscen::mezclar_fila`) y sube el RT
  (`mezclar_quad_cpu`, tope 4 MiB; si no cabe, copia opaco). Pensado para quads
  pequeños: la mezcla no se acumula porque el fondo opaco va en el mismo commit.
  La escena en CPU usa `comp_mezclar_rect`, el mismo cálculo (test
  `comp_mezcla.sal` con los vectores del test de Rust).
- **WM escena SHM:** dimensiones lógicas `sw×sh` del FB; RT GPU puede ser menor si NVDisplay
  (`comp_scanout_fb_*` en policy).
- No añadir KMS/DRM Linux; solo syscalls soso existentes.

## Errores frecuentes

- Cambiar tamaño de quad GSCEN sin actualizar kernel `GSCEN_QUAD` / tests.
- Present full-frame cuando el daño es pequeño (regresión perf; NVDisplay usa tile CE).
- Confundir puerto **7430** (clientes WM) con **7432** (compositor).
- `BIND` con `shm_map` = -1: el WM no concedió la SHM (`shm_grant` al pid de
  `tcp_peer_pid`). Si `tcp_peer_pid` da `EAGAIN` en el lado que conecta, el
  `accept` lo resolvió el planificador sin apuntar el pid del servidor
  (`tcp_loopback_fijar_servidor`). La superficie queda con stride 0 y antes
  el `commit` moría con `sal panic: bytes: out of bounds`.
