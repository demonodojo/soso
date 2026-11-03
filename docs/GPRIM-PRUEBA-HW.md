# GPRIM — prueba en hardware

`GPRIM` (GSCEN v2) pinta la escena de primitivas del escritorio con un kernel SASS propio
(`lxdde/ports/nouveau/gprim.cu`) por el canal de cómputo, sobre el RT en VRAM, y presenta el
rectángulo dañado. El oráculo es el backend CPU de `escena.sal`.

## Estado
- Validado en host (RTX 5070 Ti, NVRTC 12.8): 15 primitivas, 128x96, 0 píxeles distintos.
- Kernel (`drivers/gpu.rs`, comando `GPRIM`), puente lxdde y blobs sm_86/sm_120 escritos y
  compilados; el hostcheck C pasa. **No probado en una GPU real dentro de soso.**
- Cliente de prueba `gprim-prueba` (Sal, en `/bin`): pinta la escena en un RT de VRAM con
  `GPRIM`, lo lee con `gpu_read` y lo compara píxel a píxel con el backend CPU.
- `soso-comp` entiende `PRIMS` (op 37): recibe un comando GPRIM por el socket :7432, le pone
  RT/dimensiones/pitch, lo lanza y contesta `PRIMS_ACK` (1 pintado, 0 «pinta tú en CPU»). El primer
  rechazo del kernel se recuerda y no se reintenta. **El WM todavía no emite `PRIMS`**: el
  cromo se sigue pintando en CPU sobre el SHM del compositor y se sube con GFRAM; pintarlo en
  GPU exige decidir el orden respecto al contenido de los clientes (ver PLAN-PESTANAS G-fases).

## Pasos en la máquina con GPU
1. En el host: `scripts/gprim-hw-check.sh` (kernel contra el oráculo + blobs al día).
2. Arranca soso con GPU (`lxdde-nouveau`). En el log de serie debe aparecer
   `nouveau-lx: compute — gprim (escena 2D) disponible`. Si dice «NO disponible», el
   registro del kernel falló (`kernel_check` con 9 parámetros): revisa regcount/param_base.
3. En el guest (SSH o consola), con el escritorio **parado** (la prueba usa el canal de la GPU):
   ```
   gprim-prueba              # compara, no presenta
   gprim-prueba --presentar  # además lo presenta 3 s
   ```
   Salidas: `0` igual; `1` error de preparación; `2` sin GPU/gfx3d (no es fallo); `3` el kernel
   rechazó `GPRIM` (ENOTSUP/ENOSYS); `4` píxeles distintos (el mensaje dice cuántos).
4. Si da `4`, guarda el log de serie y `gprim-prueba` tal cual: el recuento de píxeles distintos
   y el log de `lxdde` (qué lanzó el driver) son lo que hace falta.

## Límites
≤32 primitivas, PSF ≤ 0x1F000 B, ≤128 KiB de celdas, bloque fijo de 256 hilos; `PRIMS`
admite comandos de hasta 256 KiB. Cuelgue de la GPU: el log de serie del guest dice qué lanzó;
ver skill soso-gpu.
