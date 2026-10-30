---
name: soso-wm
description: >-
  soso window manager (Sal `soso-wm`) — escritorio gráfico, PTYs, ventanas SHM,
  protocolo TCP :7430, `wm.sal`, terminales, lanzador, pruebas host/guest. Usar al
  tocar `user/sal/escritorio/soso-wm.sal`, `wm.sal`, `ventanas.sal`, `proto.sal`,
  `libsoso::ventana`, clientes gráficos (`soso-web`, `demo-cliente`) o
  [PLAN-ESCRITORIO.md](../../docs/PLAN-ESCRITORIO.md). Compositor GPU: skill soso-comp.
---

# soso — Window manager (Sal)

Gestor de ventanas en **Sal** sobre framebuffer GOP: hasta 8 terminales PTY, barra,
ratón, clientes TCP en `127.0.0.1:7430`, superficies en SHM. Plan maestro:
[`docs/PLAN-ESCRITORIO.md`](../../docs/PLAN-ESCRITORIO.md).

Seguimiento de planes: [`../soso-architecture/references/planes.md`](../soso-architecture/references/planes.md).

Build/run general: skill **`soso-dev`**. Compositor GPU (`soso-comp`, puerto 7432):
skill **`soso-comp`**.

## Mapa de fuentes

| Área | Ruta |
|------|------|
| Entrada WM | `user/sal/escritorio/soso-wm.sal` |
| Bucle eventos | `wm.sal`, `ventanas.sal`, `tabla.sal` |
| Terminal | `term.sal`, `superficie.sal`, `fuente.sal` |
| Protocolo 32 B | `proto.sal` ↔ `crates/soso-wm-proto` |
| Cliente demo | `demo-cliente.sal` |
| Explorador ficheros | `soso-files.sal`, `files_path.sal` → `/bin/soso-files` |
| Lanzador barra | `lanzador.sal` |
| Syscalls Sal | `soso.sal` (`fb_*`, `shm_*`, `tcp_*`, PTY) |
| Rust ventana | `user/libsoso/src/ventana.rs` |
| Instalación binarios | `xtask/src/sal.rs` → `rootfs/bin/soso-wm` |

## Protocolo cliente ↔ WM (TCP 7430)

- Mensajes **32 bytes** little-endian (`MSG_LEN` en `soso-wm-proto`).
- Ops principales: `HOLA`, `CREAR`, `DANIO`, `TITULO`, `CERRAR_CLI`; WM→cliente:
  `VENTANA`, `TECLA`, `RATON`, `FOCO`, `REDIM`, `CERRAR_WM`, `FOTOGRAMA`.
- Tras `CREAR`, el WM asigna SHM; el cliente pinta y notifica daño con `DANIO(x,y,w,h)`.
- **Versión** en `HOLA` (offset 4): mantener alineado Sal/Rust tests.

Tests layout Rust: `cargo test -p soso-wm-proto`.

## Kernel / FB

- Un proceso es **dueño** del modo gráfico (`SYS_FB_SET_MODE`, `fb_present` / rect).
- `Ctrl+Alt+Retroceso` mata al dueño (consola).
- Entrada exclusiva mientras el WM tiene el FB (`fb_input_exclusive`).

## Atajos de teclado (usuario)

| Atajo | Acción |
|-------|--------|
| Alt+Intro | Terminal nueva |
| Alt+Tab | Siguiente ventana |
| Alt+Q | Cerrar ventana |
| Alt+T | Flotante / mosaico |
| Alt+W | `soso-web --grafico` (página de prueba empaquetada) |
| Menú Aplicaciones → Archivos | `spawn` de `/bin/soso-files` (listado `open`/`getdents`, clic o j/k+Intro) |
| Alt+Mayús+E | Salir a consola |

## Build y pruebas

```bash
cargo xtask sal-build          # soso-wm, demo-cliente, soso-files, soso-comp → rootfs/bin
cargo xtask check              # incluye tests/sal/escritorio/probar_host.py
python3 tests/sal/escritorio/probar_host.py   # un .sal o todos
cargo test -p soso-wm-proto
cargo xtask test               # guest: init test + WM (SSH)
```

**Guest (obligatorio en CI):** en sosh **no** usar `&` en background para pruebas WM.

| Flag | Línea de éxito esperada |
|------|-------------------------|
| `soso-wm --prueba` | `soso-wm: prueba OK` |
| `soso-wm --prueba-layout` | `soso-wm: prueba-layout OK` |
| `soso-wm --prueba-web` | (W8 navegador; ver xtask test) |
| `soso-wm --fotograma` | `soso-wm: fotograma OK` |

Visual QEMU: `cargo xtask fb-shot` (skill **soso-dev**).

## Cambios habituales

1. **Nueva ventana / cliente Sal:** extender `proto.sal` + `soso-wm-proto` + handler en
   `wm.sal` o `soso-wm.sal`; añadir test host en `tests/sal/escritorio/` que importe
   el módulo bajo `user/sal/escritorio/`.
2. **Layout mosaico / hit-test:** `ventanas.sal`, `wm.sal`; `--prueba-layout` debe seguir verde.
3. **Cliente Rust:** preferir `libsoso::ventana` y el mismo layout de mensajes que `proto.sal`.
4. **Compositor GPU:** no duplicar present en el WM; escena y scanout viven en **`soso-comp`**
   (skill **soso-comp**).

## Errores frecuentes

- Olvidar `cargo xtask sal-build` tras editar `.sal` → rootfs desactualizado en QEMU.
- Romper alineación de offsets en mensajes de 32 B entre Sal y Rust.
- Pruebas guest que ejecutan `soso-wm` en background (`&`) → timeouts en xtask.
- Asumir GPU: en QEMU el compositor puede caer a blit CPU/SHM; path VRAM requiere lxdde
  (skill **soso-gpu**).
