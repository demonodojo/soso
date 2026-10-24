# Plan — Gráficos 3D y juegos nativos en soso

Objetivo: ejecutar **juegos nativos en Rust** con el **motor 3D real** de la GPU NVIDIA
(clase `BLACKWELL_B` / `0xce97` en GB205), presentando en el framebuffer GOP (y más
adelante, opcionalmente, salida HDMI de la dGPU).

**Prerrequisitos:** gates L6 G1–G5 (GSP-RM, canal GR, CE, compute) en GO en silicio;
framebuffer userspace (`SYS_FB_*`, `SYS_INPUT_POLL`); skill **`soso-gpu`**.

## Arquitectura (resumen)

- Userspace: crate **`soso-gfx`** (futuro) + juegos en `/juegos/`.
- Kernel: `SYS_GPU_SUBMIT` con comandos extendidos; puente `lx_nouveau_*` en `lxdde`.
- Driver: objeto 3D en subcanal 0 del canal GR; pushbuffer + semáforo (mismo patrón que CE/compute).

## Fases y estado

| ID | Entregable | Criterio GO | Estado |
|----|-----------|-------------|--------|
| **H1** | Objeto 3D persistente + `CLEAR_SURFACE` en VRAM + demo + hostcheck | Log `3D clear verificado (H1 GO)` en VFIO GB205; `soso-gfx-demo` mide FPS | **En curso** |
| **F1** | Presentación sin triple copia | Superficie fijada; CE VRAM→sysmem; una copia WC al GOP | Pendiente |
| **F2** | Primer triángulo | `tools/shaderc-soso` (SPIR-V→SASS); `DRAW` visible | Pendiente |
| **F3** | Crate `soso-gfx` | Búferes, RT, pipelines, batch submit | Pendiente |
| **F4** | Canal GR por proceso | GPFIFO/PB mapeados; doorbell usermode | Pendiente |
| **F5** | Entrada de juego | Ratón relativo; gamepad USB HID | Pendiente |
| **F6** | Audio de salida | Reproducción HDA | Pendiente |
| **F7** | Lanzador `soso-juegos` | Pantalla completa; juego demo 3D | Pendiente |
| **F8** | NVDisplay (opcional) | Scanout desde VRAM a HDMI dGPU | Pendiente |

## H1 — Detalle

1. Cabeceras de métodos 3D transcritas en `gsp_3d.h` (clear path).
2. `gsp_3d.c`: init/clear/fini sobre `g_chan_gr`.
3. Autoprueba 64×64 en bring-up + lectura CE.
4. Comando `GCLR3` y flag `gfx3d` en `GpuInfo`.
5. `/bin/soso-gfx-demo`, hostcheck, `g3-check`.

## Validación

```bash
./scripts/l6-g3-gsp-hostcheck.sh
cargo xtask check
cargo xtask test
# Silicio GB205:
sudo SOSO_G1_CMD='soso-gfx-demo --frames 120' ./scripts/l6-g1-vfio-test.sh
```

## Referencias

- [L6-native-autonomy.md](L6-native-autonomy.md), [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md)
- Clase 3D: `lxdde/reference/open-gpu-kernel-modules-570.144/.../class/clce97.h`
- Golden GR: `gsp_gr_golden_oneinit` en `gsp_bringup.c`
