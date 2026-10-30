# Compositor — protocolo WM ↔ soso-comp

Puerto TCP **7432** (`comp_puerto()` / `soso-wm-proto::comp::PORT`).

## Mensajes WM → compositor (32 B, `comp_proto.sal`)

| op | Valor | Uso |
|----|-------|-----|
| `COMP_ESCENA` | 30 | Tras conectar: SHM del framebuffer lógico + metadatos |
| `COMP_FRAME` | 31 | Present rect dañado (legacy / sin escena GPU) |
| `COMP_BIND` | 33 | Superficie cliente → textura |
| `COMP_GEOM` | 34 | Posición, z, flags (`GEOM_HIDDEN`, `GEOM_OPAQUE`) |
| `COMP_DMAP` | 35 | Copia SHM cliente → textura |
| `COMP_COMMIT` | 36 | Commit escena; flag `COMMIT_CLEAR_RT` |

## Pipeline GPU (`comp_escena.sal`)

1. `BIND` — id, shm, w, h, stride  
2. `GEOM` — x, y, w, h, z, flags  
3. `DMAP` — subrect de la superficie  
4. `COMMIT` — rect de daño + opcional clear RT  

El compositor construye **`GSCEN`** (kernel `gpu.rs`): header 53 B + hasta 19 quads de 36 B.
Cada quad: textura VA, dest, tamaño, **sx/sy** origen en textura (clip).

Recorte: `comp_escena_commit` intersecta con rect COMMIT y omite quads tapados por opacos
con z mayor (`comp_clip.sal`, tests `comp_gscen_clip.sal`).

## Fallback sin `gfx3d`

`comp_st_gpu_ok() == 0` → `fb_present_rect` sobre SHM del compositor.

## Scanout

Con GPU: `GSCEN` present → router F8 (`kernel/src/drivers/scanout.rs`). RT acotado a head
NVDisplay vía `GpuInfo._pad[3]` (`comp_scanout_policy.sal`). Detalle: skill **soso-gpu** /
`docs/PLAN-NVDISPLAY.md`.

## Tests

| Test Sal | Qué valida |
|----------|------------|
| `comp_proto.sal` | Ops y encoders |
| `comp_gscen_layout.sal` | Tamaño quads / buffer |
| `comp_gscen_clip.sal` | Recorte commit |
| `comp_scanout_policy.sal` | Política RT vs head |
| `tests/sal/escritorio/comp_*.sal` | Importan `user/sal/escritorio/` |

Rust: `cargo test -p soso-wm-proto` (`gscen`, `gfram`, `gpres`, `nvdisp`, `scanout`).
