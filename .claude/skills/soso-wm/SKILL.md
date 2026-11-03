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
| Tema / escala | `tema.sal`, `fuentes.sal`, `zoom.sal` |
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
  `VENTANA`, `TECLA`, `RATON`, `FOCO`, `REDIM`, `CERRAR_WM`, `FOTOGRAMA`, `TEMA` (23),
  `ESCALA` (24), `VISIBLE` (25: u32 @4 = 1 se ve / 0 oculto). `HOLA` lleva `app_id` @8 (1 web, 2 archivos). `VENTANA`: tema u32 @24, escala inicial @28.
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
| Alt+Q | Cerrar pestaña activa (la ventana si es la última) |
| Alt+Mayús+Q | Cerrar ventana con todas sus pestañas |
| Ctrl+Mayús+T | Nueva pestaña con la misma aplicación |
| Ctrl+Tab / Ctrl+Mayús+Tab | Pestaña siguiente / anterior |
| Alt+1…9 | Ir a la pestaña n |
| Alt+T | Flotante / mosaico |
| Alt+W | `soso-web --grafico` (página de prueba empaquetada) |
| Alt+M | Salvapantallas (mono) |
| Alt+D / menú «Modo claro/oscuro» | Alterna `tema.sal` (repinta + `op TEMA` a clientes) |
| Ctrl+/- | Escala fuente **solo ventana con foco** (`tab_escala`, `op ESCALA`) |
| Ctrl+Mayús+/- | Escala **solo WM** (barra/títulos; `REDIM` a clientes si cambia barra) |
| Menú Aplicaciones → Archivos | `spawn` de `/bin/soso-files` (listado `open`/`getdents`, clic o j/k+Intro) |
| Alt+Mayús+E | Salir a consola |

**Ratón sobre ventanas:** barra de título = arrastrar (clamp para que no se pierda bajo la barra
superior); doble clic = maximizar/restaurar; botones (derecha→izquierda) cerrar, maximizar,
minimizar (`ventana_hit_boton`); bordes derecho/inferior/izquierdo y esquinas inferiores
redimensionan (`ventana_hit_borde`, 5 px; 14 px en esquinas). Una maximizada no se arrastra ni
redimensiona. Las minimizadas siguen en la lista de ventanas de la barra superior: clic =
restaurar (o minimizar si ya tiene el foco). Estado por ventana en `tab_estado()` + geometría
guardada `tab_sx/sy/sw/sh`; la ranura mide 272 B.

## Pestañas (grupo/panel)

Una ventana es un **grupo** (`tabla.sal`: `grupo_cab`, `grupo_integrar`, `grupo_separar`,
`grupo_panel_activo`); cada pestaña es un **panel** (slot). La geometría y el estado
(`tab_wx…tab_sh`, `tab_estado`) viven en la cabecera del grupo y `tab_get/tab_set` los
redirigen. El z-order y el foco sólo contienen cabeceras; el contenido (teclas, ratón,
blit) es el del panel activo. Arrastre de pestañas: `wm_pest_arrastrar` / `wm_pest_despegar` / `wm_destino_soltar`; menú contextual (clic derecho): `ctx_*`, `wm_ctx_*`. Atajos: Ctrl+Mayús+T nueva, Alt+Mayús+D sacar, Alt+Mayús+H/L mover. El cromo se pinta con la escena de primitivas (`escena.sal`); GPU: `tools/gpu-prim/validar.py`. Plan y estado: [`docs/PLAN-PESTANAS.md`](../../docs/PLAN-PESTANAS.md).

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

**Mirar el escritorio sin ventana:** `scripts/wm-probe.py` arranca soso con monitor y QMP
(`SOSO_QEMU_MONITOR`, `SOSO_QEMU_QMP` en `cargo xtask run`), abre `soso-wm` y ejecuta pasos
(`key:alt-ret`, `type:`, `move:X,Y`, `click:X,Y`, `down`/`up`, `wait:`, `shot:nombre`, `pix:X,Y`);
deja PNG en `target/wm-probe/`. Ejemplo: `scripts/wm-probe.py click:50,12 move:60,75 shot:menu`.
Con él se vio que el cursor estaba bien y que el menú se pintaba en (0,0) en vez de bajo la barra.
Ojo: los PNG se leen con la herramienta Read; GTK/Wayland no hace falta para nada.

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
- `wm_drenar_input` NO debe volcar `st` a la tabla tras procesar la entrada: deshacía cada arrastre.
- El repintado completo es caro: `fuente_dibujar` relleno por celda y rectángulos redondeados por filas
  (de ~750 ms a ~80 ms por fotograma); no vuelvas a píxel-a-píxel en rutas calientes.
- Romper alineación de offsets en mensajes de 32 B entre Sal y Rust.
- Pruebas guest que ejecutan `soso-wm` en background (`&`) → timeouts en xtask.
- Asumir GPU: en QEMU el compositor puede caer a blit CPU/SHM; path VRAM requiere lxdde
  (skill **soso-gpu**).
