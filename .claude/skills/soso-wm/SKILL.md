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
| Letra TrueType | `ttf.sal` (tablas, rasterizado, caché), `fuentes.sal` (cachés por nivel), `rootfs/lib/fonts/DejaVuSans{,Mono}.ttf` |
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
- `Ctrl+Alt+Retroceso` mata al grupo del dueño y vuelve la consola (PS/2 y USB, Alt izquierdo con cualquier Ctrl).
- **Vigilante del dueño**: `wm_tick` llama a `fb_keepalive(5000)` (`SYS_FB_KEEPALIVE=104`).
  Si pasan 5 s sin señal, `poll_fb_watchdog` (scheduler) mata con `SIGKILL` **sólo el
  grupo del dueño** y `release_owner_if` devuelve la consola: un WM colgado no se lleva
  la shell. Un tick normal son 10 ms; no subir de ~5 s el plazo sin medirlo.
- **La consola vuelve con cualquier final del dueño**: muerte por señal, `exit` normal,
  falta (255) o pánico (101); `exit_current` también llama a `release_owner_if`. Antes
  sólo se soltaba por señal, y un WM que fallaba dejaba la pantalla congelada con el
  teclado en exclusiva para un proceso muerto. Red de seguridad en el scheduler
  (`soltar_pantalla_sin_dueno`): un dueño que no vive o es zombi se suelta solo.
  `fb_info` marca el modo gráfico con `FB_INFO_GRAFICO` en `_pad` (prueba en `init test`).
- El WM no arranca (rc 5) si `fb_set_mode` falla porque otro tiene la pantalla, y
  devuelve la consola en todos sus caminos de salida (también sin puerto, rc 4).
- **Ctrl-C con la pantalla en exclusiva** va sólo al dueño (`FB_SIGINT` →
  `signal_one(owner, SIGINT)`), no al grupo en primer plano de la consola. El WM lo
  ignora (`senal_ignorar(sig_int())` en `bucle_grafico`) y reenvía a la ventana con foco
  los códigos de control que no son atajos suyos (`wm_ctrl_para_foco`: todo 1–31 salvo
  Ctrl+Tab y Ctrl+Mayús+T). Así Ctrl-C interrumpe en la terminal sin cerrar el
  escritorio ni matar al compositor y las apps de su grupo; un programa a pantalla
  completa que no lo ignore (`soso-web --grafico`) sigue saliendo con Ctrl-C.
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

**`soso-wm --sin-comp`**: escritorio normal sin compositor. `wm_comp_ensure` devuelve -1 sin
lanzar `soso-comp` ni conectarse a uno vivo; con `comp_fd < 0` no hay modo GPU
(`wm_comp_gpu_mode` = 0) y `wm_comp_flush` presenta con `fb_present_rect` (GOP). Es la
salida para descartar la GPU/NVDisplay en placa.

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

## Repintado: daño por región (CPU)

Quien cambia algo **declara el rectángulo dañado** con `wm_danar(st, x0,y0,x1,y1)`
(o `wm_danar_ventana` con sombra, `wm_danar_cabecera`, `wm_danar_barra`,
`wm_danar_menu`, `wm_danar_ctx`) en vez de `wm_pedir_repintar`, que repinta la
pantalla entera. Así lo hacen wlroots, KWin y Mutter.

- Hasta `wm_reg_max()` = 4 rectángulos en `reg_buf`. Los que se tocan se funden; si
  no caben, se funde con el que menos crezca; si el total pasa de media pantalla,
  se pide el repintado completo.
- `wm_repintar_region` pinta fondo → barra → ventanas en orden z (marco con
  sombra + contenido) en un **búfer auxiliar de stride = ancho de la región** y lo
  copia a `compos`. Todos los pintores recortan contra `stride` y la longitud del
  búfer, así que nada se sale y las mezclas con alfa caen sobre fondo recién
  pintado. La rejilla de terminal se pinta con `force` sin consumir su marca de
  filas sucias (se guarda y restaura).
- Una región que toca la barra se amplía a la franja completa de arriba
  (`0,0,sw,…`): la barra se pinta a coordenadas absolutas. Una que toca el menú
  contextual lo cubre entero (sus esquinas se mezclan y se repinta cada fotograma).
- Sólo CPU. Con compositor GPU o salvapantallas, `wm_danar` cae al repintado completo.
- Comprobar un cambio: captura tras la acción, `Alt+D` dos veces (repintado
  completo con el mismo tema) y otra captura: **0 píxeles distintos**.
- Coste en QEMU (TCG): hover de cabecera 2–3 ms, arrastre de una terminal
  13–17 ms, repintado completo ~25 ms.

Rellenar: `bytes_fill_u32` del runtime de Sal escribe de 4 en 4 bytes y es unas
30 veces más lento que `bytes_copy` (memcpy). `rellenar_rect` rellena 16 píxeles,
los duplica con copias dentro de la fila (`fila_rellenar_u32`) y copia esa fila a
las demás. No vuelvas a llamar a `bytes_fill_u32` por filas en rutas calientes.

## Errores frecuentes

- Olvidar `cargo xtask sal-build` tras editar `.sal` → rootfs desactualizado en QEMU.
- `wm_drenar_input` NO debe volcar `st` a la tabla tras procesar la entrada: deshacía cada arrastre.
- El repintado completo es caro: `fuente_dibujar` relleno por celda y rectángulos redondeados por filas
  (de ~750 ms a ~80 ms por fotograma); no vuelvas a píxel-a-píxel en rutas calientes.
- Letra: TrueType rasterizada en tiempo de ejecución (`ttf.sal`, algoritmo de font-rs en
  `Float`; sin libm: `floor`/`ceil`/`abs` a mano). `Fuentes.cargar(nivel)` crea una caché en
  modo celda de DejaVu Sans Mono (el avance de «M» llena la celda; cajas U+2500–259F
  estiradas de borde a borde) y su puntero circula como «psf»: `fuente_dibujar` la reconoce
  por la marca `TTFC`. `Fuentes.ui(nivel)` es DejaVu Sans proporcional (con kerning) para
  barra, títulos y menús (`wm_texto`, `wm_texto_titulo`), mezclada con lo ya pintado. Cada
  glifo se rasteriza una vez por caché. Sin los .ttf se cae a las Spleen. Compararlo con
  FreeType: `python3 tools/ttf-prueba/comparar.py` (PNG en `target/ttf-prueba/`). Ojo: las
  `spleen-6x12/16x32/32x64` son PSF2 estándar que `fuente.sal` lee mal y `spleen-12x24`
  está corrupta. En Sal, una expresión que empieza por `(` tras un bloque se lee como
  llamada al bloque («only direct calls supported»): asígnala antes a una variable.
- Esquinas suavizadas: la cobertura de un píxel es `rr_cov16` (`superficie.sal`, 0..16);
  la CPU la aplica por tramos de fila y `lxdde/ports/nouveau/gprim.cu` por píxel, con la
  misma aritmética entera. Tocar una sin la otra rompe `python3 tools/gpu-prim/validar.py`
  (dos oráculos: fuente de 1 bit y de cobertura; tiene que dar 0 píxeles distintos) y
  después hay que regenerar el SASS con `python3 tools/gpu-prim/cubin.py`.
- Sombra de ventanas: primitiva `esc_k_sombra` (distancia al rectángulo redondeado),
  sólo sobre fondo recién pintado (repintado completo o región). `wm_pintar_marco_s(…, 0)`
  al repintar una ventana suelta sin fondo: se mezcla con lo que hay debajo y repetirla
  la oscurece.
- Romper alineación de offsets en mensajes de 32 B entre Sal y Rust.
- Pruebas guest que ejecutan `soso-wm` en background (`&`) → timeouts en xtask.
- Asumir GPU: en QEMU el compositor puede caer a blit CPU/SHM; path VRAM requiere lxdde
  (skill **soso-gpu**).
- Cursor en los bordes: `wm_pintar_cursor_buf` debe recortar ancho/alto como la
  captura y restauración del fondo. Dibujar el cursor completo en el extremo
  derecho/inferior cruza filas o termina en `bytes: out of bounds` y mata el WM.
  Regresión host: `tests/sal/escritorio/cursor_bordes.sal`.
- Para comprobar entrada desde fuera de QEMU, usar la tableta USB absoluta.
  PS/2 es relativo y no sincroniza la posición del cursor del host al entrar.
  QMP no sustituye comprobar salida/reentrada y pérdida/recuperación de foco
  de la ventana GTK; verificar también las cuatro esquinas.
- Terminal vacía al abrir: comprobar que `spawn_io_full` inicialice a cero todos
  los campos opcionales de `SpawnIo` (incluido `cwd_len`) y que el WM no integre
  un panel si `spawn_io_full` devuelve error. `term_capa` debe devolver el `Bytes`
  dueño, no sólo su puntero, y cada panel debe conservar ese dueño. Regresiones:
  `tests/sal/escritorio/term.sal` más apertura, escritura y cambio entre dos PTY
  en QEMU; confirmar que el contenido inicial siga visible al volver al panel.
