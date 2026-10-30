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
| `GCLR3` | Clear RT (flag commit) |
| `GPRES` / `GNVD` | Present explícito (demo; WM usa router vía `GSCEN`) |

Longitudes y offsets: módulos `comp_*_layout.sal` y submódulos en `soso-wm-proto`.

## Pruebas

```bash
python3 tests/sal/escritorio/probar_host.py comp_gscen.sal comp_gscen_clip.sal comp_scanout_policy.sal
cargo test -p soso-wm-proto
cargo xtask check
./scripts/l6-scanout-regression.sh   # política gop/nvdisp (F8)
```

Integración guest con GPU real: VFIO + `gfx3d` (skill **soso-gpu**); QEMU suele quedarse en GOP.

## Convenciones

- **Daño mínimo:** COMMIT lleva rect; GSCEN solo incluye quads visibles tras clip.
- **Opaco:** barra/menú/ventanas sólidas usan `GEOM_OPAQUE`; cursor no.
- **WM escena SHM:** dimensiones lógicas `sw×sh` del FB; RT GPU puede ser menor si NVDisplay
  (`comp_scanout_fb_*` en policy).
- No añadir KMS/DRM Linux; solo syscalls soso existentes.

## Errores frecuentes

- Cambiar tamaño de quad GSCEN sin actualizar kernel `GSCEN_QUAD` / tests.
- Present full-frame cuando el daño es pequeño (regresión perf; NVDisplay usa tile CE).
- Confundir puerto **7430** (clientes WM) con **7432** (compositor).
