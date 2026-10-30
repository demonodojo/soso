# Plan — Gráficos 3D y juegos nativos en soso

Objetivo: ejecutar **juegos nativos en Rust** con el **motor 3D real** de la GPU NVIDIA
(clase `BLACKWELL_B` / `0xce97` en GB205), presentando en **GOP** (QEMU/desarrollo) o
**NVDisplay** (VRAM→HDMI dGPU cuando el head está GO) vía el mismo router que el compositor.

**Prerrequisitos:** gates L6 G1–G5 (GSP-RM, canal GR, CE, compute) en GO en silicio;
framebuffer userspace (`SYS_FB_*`, `SYS_INPUT_POLL`); skill **`soso-gpu`**.

## Arquitectura (resumen)

- Userspace: crate **`soso-gfx`** (futuro) + juegos en `/juegos/`.
- Kernel: `SYS_GPU_SUBMIT` con comandos extendidos; puente `lx_nouveau_*` en `lxdde`.
- Driver: objeto 3D en subcanal 0 del canal GR; pushbuffer + semáforo (mismo patrón que CE/compute).

## Fases y estado

| ID | Entregable | Criterio GO | Estado |
|----|-----------|-------------|--------|
| **H1** | Objeto 3D persistente + `CLEAR_SURFACE` en VRAM + demo + hostcheck | Log `3D clear verificado (H1 GO)` en VFIO GB205; `soso-gfx-demo` mide FPS | **Implementado** (hostcheck + demo); GO silicio = VFIO GB205 |
| **F1** | Presentación sin triple copia | Superficie fijada; CE VRAM→sysmem; una copia WC al GOP; `GPRES` + hostcheck | **Implementado** (hostcheck); GO silicio con `soso-gfx-demo` en VFIO |
| **F2** | Primer triángulo | `tools/shaderc-soso` (SPIR-V→SASS); `DRAW` visible | Pendiente |
| **F3** | Crate `soso-gfx` | Búferes, RT, pipelines, batch submit | Pendiente |
| **F4** | Canal GR por proceso | GPFIFO/PB mapeados; doorbell usermode | Pendiente |
| **F5** | Entrada de juego | Ratón relativo; gamepad USB HID | Pendiente |
| **F6** | Audio de salida | Reproducción HDA | Pendiente |
| **F7** | Lanzador `soso-juegos` | Pantalla completa; juego demo 3D | Pendiente |
| **F8** | NVDisplay + router scanout | `GNVD` / `GSCEN`→HDMI o GOP; N0–N8 en host | **Implementado (host)** — [PLAN-NVDISPLAY.md](PLAN-NVDISPLAY.md); **GO silicio HDMI** = patrón/foto + `hw-matrix` (canal disp HW aún best-effort) |

## H1 — Detalle

1. Cabeceras de métodos 3D transcritas en `gsp_3d.h` (clear path).
2. `gsp_3d.c`: init/clear/fini sobre `g_chan_gr`.
3. Autoprueba 64×64 en bring-up + lectura CE.
4. Comando `GCLR3` y flag `gfx3d` en `GpuInfo`.
5. `/bin/soso-gfx-demo`, hostcheck, `g3-check`.

## F1 — Detalle

1. Shadow del GOP fijado; sus páginas se mapean en el vaspace GPU (`G6_SRC_VA`).
2. `gsp_buf_download_dma`: CE VRAM → sysmem en trozos (mismo límite que G6).
3. Comando `GPRES` en `SYS_GPU_SUBMIT` (handle RT + w/h/pitch); el kernel vuelca shadow→GOP (`movntdq`).
4. `soso-gfx-demo`: `GCLR3` + `GPRES` por frame (sin `gpu_read` ni `fb_present`).
5. Hostcheck: codificación del pushbuffer de presentación.

**Scanout GOP:** la consola y `soso-web` siguen usando `fb_present` al shadow; el compositor
(`soso-comp`, `GSCEN`) usa el router F8 cuando `gfx3d=1`.

## F8 — Detalle (NVDisplay)

Plan maestro: [PLAN-NVDISPLAY.md](PLAN-NVDISPLAY.md) (fases N0–N8 **cerradas en host**).

1. **Router** (`kernel/src/drivers/scanout.rs`): `SOSO_SCANOUT=auto|gop|nvdisp` en build;
   log `scanout: policy=… effective=… mode=…`.
2. **Comandos:** `GPRES` (GOP explícito), `GNVD` (25 B, mismo layout), `GSCEN`/`GFRAM`
   eligen backend al presentar.
3. **`GpuInfo._pad`:** `[0]` gfx3d, `[1]` nvdisp capaz, `[2]` gop, `[3]` backend efectivo.
4. **Port:** `nvdisp_*` + `engine/disp` acotado; GB205 comparte rutas ga102/gf119;
   scanout tile vía `gsp_vram_blit_pitch` (CE), fallback PRAMIN.
5. **Userspace:** `soso-gfx-demo --scanout auto|gop|nvdisp`; `soso-comp` acota RT a 640×480
   si backend efectivo es NVDisplay.

**Criterio GO F8 en silicio** (VFIO/nativo + monitor en la dGPU):

- Escritorio o demo con `SOSO_SCANOUT=auto` usa NVDisplay en el hot path (log + `_pad[3]`).
- Regresión `SOSO_SCANOUT=gop` y `cargo xtask fb-shot` en QEMU sin rotura.
- Evidencia por SKU en `docs/hw-matrix.json`.

**Host / CI:**

```bash
./scripts/l6-scanout-regression.sh
./scripts/l6-nvdisp-smoke.sh
./scripts/l6-nvdisp-blackwell.sh
```

Audio HDMI (`10de:2f80`): mismo grupo IOMMU que la dGPU en VFIO (`l6-g1-preflight` §2b);
reproducción = **F6**, no F8.

## Validación

```bash
./scripts/l6-g3-gsp-hostcheck.sh
./scripts/l6-scanout-regression.sh   # F8 política gop/auto
cargo xtask check
cargo xtask test
# Silicio GB205 (GOP o NVDisplay según build/capacidades):
sudo SOSO_G1_CMD='soso-gfx-demo --frames 120 --scanout auto' ./scripts/l6-g1-vfio-test.sh
```

## Referencias

- [PLAN-NVDISPLAY.md](PLAN-NVDISPLAY.md) — F8 / scanout unificado
- [L6-native-autonomy.md](L6-native-autonomy.md), [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md)
- Clase 3D: `lxdde/reference/open-gpu-kernel-modules-570.144/.../class/clce97.h`
- Golden GR: `gsp_gr_golden_oneinit` en `gsp_bringup.c`
