# Plan: pestañas en `soso-wm` y escritorio sobre GPU

Estado (2026-11-03): **B1, T0–T4 hechas** (T4 sin animaciones ni flechas de desbordamiento); **G1, G2 y G4 validadas en GPU del host** (kernel `tools/gpu-prim/gprim.cu`), pendiente su integración en el kernel de soso; G3, G5 y T7 sin empezar.
Relacionado: [`PLAN-ESCRITORIO.md`](PLAN-ESCRITORIO.md) (W1–W10), [`PLAN-NVDISPLAY.md`](PLAN-NVDISPLAY.md) (F8, compositor), skills `soso-wm`, `soso-comp`, `soso-gpu`.

## 1. Objetivo

1. Una **ventana** puede contener varias **pestañas**; cada pestaña es una aplicación (terminal, cliente TCP como `soso-web`/`soso-files`).
2. **Integrar** una ventana en otra como pestaña (arrastrando) y **sacar** una pestaña a ventana propia.
3. **Nueva pestaña con la misma aplicación** que la pestaña activa (otra terminal, otro `soso-files`, otro `soso-web`).
4. Aspecto cuidado y cómodo: barra de pestañas integrada en la barra de título, animaciones cortas, arrastre con vista previa, indicadores de actividad.
5. **Todo lo que se pueda, en GPU**: el WM deja de pintar píxeles del «cromo» (marcos, sombras, degradados, esquinas, texto de títulos) y pasa a describir una escena de primitivas que ejecuta un backend GPU o, si no hay GPU, un backend CPU idéntico en resultado.

Nombre en el código: ya existe `tab` como *tabla de ventanas* (`tabla.sal`). Para evitar confusión, el concepto nuevo se llama **grupo** (la ventana visible) y **panel** (el contenido de una pestaña = el slot actual de `tabla.sal`). «Pestaña» queda para el usuario y la documentación.

## 2. Modelo de datos

```
grupo (ventana)                      panel (slot actual de tabla.sal)
  geometría: wx wy ww wh                tipo (term/cli), fd, pid, shm, cw ch
  estado: normal/max/min + sx..sh       título, escala, term_st
  z-order                               grupo_id, orden_en_grupo
  paneles[0..N], activo                 hash/cmd de lanzamiento (ver §4)
  foco, hover de pestaña                «actividad pendiente» (bit)
```

- La **geometría y el estado** (max/min, restaurar, mosaico) pasan del slot al grupo. El slot conserva sólo el tamaño de contenido (`cw/ch`).
- El z-order, el mosaico (Alt+T), la lista de la barra superior y el foco operan sobre **grupos**.
- Un grupo con 1 panel se ve exactamente como hoy (título centrado, sin tira de pestañas).
- Límites: revisar `tab_cap` (8 PTY), `comp_surf_max` y `st_size` (432); subir a 16 paneles y 16 grupos.
- Sólo el panel activo se compone y recibe ratón/teclado. Los demás: `GEOM_HIDDEN` en el compositor y no se repintan.

## 3. Interacción

| Gesto | Efecto |
|-------|--------|
| Clic en pestaña | activar |
| Clic en «+» (o **Ctrl+Mayús+T**) | nueva pestaña **misma aplicación** que la activa |
| Clic en × de la pestaña / clic central / **Alt+Q** | cerrar pestaña (si es la última, cierra la ventana) |
| **Alt+Mayús+Q** | cerrar ventana entera |
| **Alt+1…9**, **Ctrl+Re Pág / Av Pág** | ir a pestaña / anterior-siguiente |
| **Alt+Mayús+Re Pág / Av Pág** | mover la pestaña a izquierda/derecha |
| **Alt+Mayús+D** | sacar la pestaña a ventana propia |
| Arrastrar pestaña dentro de la tira | reordenar (las vecinas se apartan animadas) |
| Arrastrar pestaña fuera de la tira | se despega: aparece una ventana «fantasma» que sigue al ratón y, al soltar, se materializa |
| Arrastrar pestaña sobre la tira de otra ventana | se resalta el hueco de inserción; al soltar, se integra |
| Arrastrar una **ventana** entera por su barra de título hacia la tira de otra | integra todas sus pestañas |
| Doble clic en zona vacía de la tira | maximizar/restaurar (como hoy en el título) |
| Arrastrar zona vacía de la tira | mover ventana |
| Clic derecho en pestaña | menú: duplicar, sacar, cerrar otras, cerrar a la derecha |

Los atajos con Alt son del WM; Ctrl+… sólo se interceptan los de pestañas (resto pasa a la app). Alt+Tab sigue cambiando de **ventana**; Alt+Tab mantenido muestra grupos. *Decidido por defecto; se cambian en `wm_atajo_*` sin tocar el modelo.*

## 4. «Misma aplicación» (nueva pestaña)

- Terminal: otro PTY + `sosh` (ya existe `wm_nueva_terminal`). Opcional: heredar el directorio actual si el kernel lo expone.
- Clientes lanzados por el WM (`wm_lanzar_web`, `wm_lanzar_ficheros`, lanzador): el WM guarda el **descriptor de lanzamiento** (ruta + argumentos) en el panel y lo repite.
- Clientes que se conectan solos al 7430: ampliar `HOLA` con un identificador de aplicación (`app_id`: cadena corta o hash) y un campo «argv reutilizable»; si no lo traen, el «+» queda deshabilitado para ese panel (no se inventa un lanzamiento).
- `soso-web` ya tiene 4 pestañas internas (W8). Por defecto **coexisten** (las del navegador siguen dentro del cliente). Fase opcional T7: el navegador pide al WM una pestaña de WM por pestaña suya y dibuja sólo el contenido.

## 5. Diseño visual

Tira de pestañas **integrada en la barra de título** (altura 30 px a escala 1; escala con `zoom.sal`):

- Pestaña activa: mismo color que el cuerpo de la ventana (se funde), esquinas superiores radio 8, línea de acento de 2 px arriba, texto en color de primer plano.
- Inactivas: fondo más oscuro/translúcido, texto atenuado; al pasar el ratón se aclaran y aparece ×.
- Anchura: máx 200 px, mín 72 px; se reparte equitativamente; con desbordamiento aparecen flechas de desplazamiento y un menú desplegable con la lista.
- Título con elipsis («…» no existe en Spleen: se pinta como `...` con ancho recortado), icono de 14 px por tipo de aplicación (terminal, web, archivos) y **punto de actividad** en pestañas inactivas con salida nueva o campana.
- Botón «+» tras la última pestaña; controles de ventana a la derecha como hoy.
- Arrastre: la pestaña levantada se dibuja con sombra y alfa ~85 %; hueco de inserción con barra de acento; al despegar, miniatura (textura reducida) de la ventana.
- Animaciones de 120–160 ms con ease-out: abrir/cerrar pestaña, reordenar, cambiar de activa (fundido corto), minimizar hacia el botón de la barra superior. Un reloj de fotogramas en el WM (`uptime_ms`) las conduce; sin GPU se degradan a salto directo si un fotograma supera el presupuesto (hoy un repintado completo en CPU cuesta ~80 ms, así que en CPU sólo se repinta la tira).
- Todos los colores, radios y tiempos van a `tema.sal` (tokens), claro y oscuro.

## 6. Arquitectura GPU

Hoy el compositor (`soso-comp`, `GSCEN`) sólo sabe pintar **quads con textura** + opaco/oculto + clear; la CPU pinta cada superficie y el cromo (marcos, sombras, degradados, texto). Se quiere que el cromo y el texto del WM salgan de la CPU.

**Principio: una escena de primitivas, dos backends.** El WM emite una lista de primitivas; el backend CPU (rasterizador en Sal, el de hoy refactorizado) y el backend GPU (comando `GSCEN` ampliado) la interpretan con el mismo resultado. Así la semántica se prueba en QEMU y en host con el backend CPU, y el GPU se valida comparándolo con él.

Primitivas nuevas (versión 2 de `GSCEN`; el quad actual pasa a ser un tipo más):

| Primitiva | Sustituye a |
|-----------|-------------|
| `SOLID` rect color+alfa | `rellenar_rect(_alfa)` |
| `GRAD` degradado vertical/diagonal | `degradado_vertical`, fondo, títulos |
| `RRECT` rect redondeado con radio por esquina, relleno, borde 1 px (SDF con antialiasing) | `rellenar_rect_redondeado(4)`, marcos, pestañas, botones |
| `SHADOW` sombra de caja analítica (radio de desenfoque, desplazamiento) | `sombra_anillo` |
| `GLYPHS` tira de glifos desde un atlas alfa en VRAM + color | `fuente_dibujar` en barra, menú, títulos, pestañas |
| `TEXQUAD` con alfa, escala y recorte | quads actuales; fundidos, miniaturas de arrastre |
| `BLUR` (opcional) desenfoque de fondo bajo barra/menú translúcidos | pre-mezcla actual contra el fondo |

Además:
- **Atlas de glifos**: los 5 tamaños Spleen se rasterizan una vez a una textura alfa (U+0000–U+00FF + puntuación usada); cambio de tema/escala sólo cambia el color o el tamaño de atlas.
- **Terminal en GPU** (fase posterior): la superficie del terminal pasa a ser una **rejilla de celdas** (cp, fg, bg, atributos) en SHM y un único dibujado instanciado con el atlas; scroll y cursor dejan de tocar píxeles.
- **Daño mínimo**: cada primitiva lleva su caja; se mantiene el recorte por z-order/opacidad actual.
- **Ritmo de fotogramas**: presentar sincronizado con el barrido vertical cuando el scanout lo permita; la animación usa el reloj de fotogramas.
- **Cursor**: investigar el canal de cursor de hardware de NVDisplay para quitarlo del compositor.
- Fuera de alcance por ahora: clientes renderizando directamente en texturas VRAM (sin copia SHM→VRAM); requiere syscall de reserva de VRAM.

**Qué puedo y qué no puedo verificar aquí.** El QEMU de este equipo no tiene GPU. En QEMU se validan: modelo de grupos, hit-testing, interacción, backend CPU y el *formato* de la escena (layouts Sal/Rust). El código de GPU (kernels, ampliación del handler `GSCEN` del kernel, atlas, SDF) sólo se valida en tu hardware (VFIO + `gfx3d`, p. ej. la GB205). Cada fase GPU se entrega con un guion de comprobación para ese hardware y comparación de capturas contra el backend CPU. Los kernels hay que compilarlos por arquitectura (sm_86 / sm_120): vía Docker/nvcc o NVRTC→PTX, a decidir en G1 según lo que ya usa `GSCEN`.

## 7. Fases

Pista A = pestañas; pista B = GPU. B1 va antes que la UI nueva para que las pestañas nazcan ya sobre primitivas.

| Fase | Contenido | Verificación |
|------|-----------|--------------|
| **T0** (hecha) | Modelo grupo/panel: mover geometría y estado de `tabla.sal` al grupo; adaptar `ventanas.sal`, mosaico, lista de barra, foco; sin cambios visibles | host `ventanas.sal`/`tabla.sal`; `--prueba`, `--prueba-layout`; wm-probe: todo lo hecho en W10 sigue igual |
| **B1** | Capa de escena de primitivas con backend CPU: formato, refactor del cromo actual (marco, botones, barra, menú) para emitir primitivas; `GSCEN` v2 sólo como layout (Sal + `soso-wm-proto` + tests) | host: oráculo de píxeles vs. captura actual; wm-probe `pix:` |
| **T1** (hecha) | Tira de pestañas (CPU/primitivas): crear, activar, cerrar, «+», misma aplicación para terminal y clientes lanzados por el WM, atajos de §3 | `soso-wm --prueba-pestanas`; wm-probe con capturas |
| **T2** | Arrastre: reordenar, despegar, integrar (pestaña y ventana entera), hueco de inserción, ventana fantasma | wm-probe `down/move/up` + capturas |
| **T3** | Protocolo: notificación de visibilidad a clientes (panel oculto no repinta), `app_id` en `HOLA`, actividad en inactivas; límites de §2 | `cargo test -p soso-wm-proto`; guest test |
| **T4** | Acabado: elipsis, desbordamiento con flechas/menú, menú contextual, iconos, animaciones con reloj de fotogramas, claro/oscuro | capturas en ambos temas, a 2 escalas |
| **G1** | Backend GPU de `SOLID/GRAD/RRECT/SHADOW` y alfa en quads: ampliar `GSCEN` en kernel y compositor | **en hardware**: guion + comparación con backend CPU |
| **G2** | Atlas de glifos + `GLYPHS`; títulos, barra, menú y pestañas sin texto en CPU | hardware |
| **G3** | Animaciones y arrastre en GPU (miniaturas, fundidos, sombra de la pestaña levantada); sincronización vertical | hardware (fluidez) |
| **G4** | Terminal por rejilla de celdas en GPU | hardware + prueba de terminal en QEMU con backend CPU |
| **G5** | Opcional: blur de fondo, cursor de hardware | hardware |
| **T7** | Opcional: `soso-web` usa pestañas del WM | guest |

Orden de ejecución sugerido: T0 → B1 → T1 → T2 → T3 → T4; G1–G3 en cuanto B1 esté cerrada y haya acceso a la placa; G4/G5 y T7 al final.

## 8. Riesgos

- **Mover la geometría al grupo toca casi todo `wm.sal` (≈96 KB)**: se hace en T0 sin cambios visibles y con los tests de W10 como red.
- **Rendimiento CPU**: hoy un repintado completo son ~80 ms; las animaciones no son viables sin repintado parcial. En CPU sólo se anima la tira y se cae a salto directo si no hay presupuesto.
- **GSCEN v2 rompe el wire format** (kernel + `soso-wm-proto` + layouts Sal): se versiona y se cambia todo a la vez, como pide la skill `soso-comp`.
- **Sin GPU en QEMU**: el trabajo B2+ no puede darse por bueno hasta probarlo en placa; las fases GPU se cerrarán sólo con tu confirmación.
- **Clientes sin `app_id`**: «+» deshabilitado en lugar de adivinar.
- **Colisión de atajos**: Ctrl+Mayús+T y Ctrl+Re Pág pueden pisar atajos de apps (el navegador); si ocurre, el WM deja pasar las teclas a los paneles de tipo cliente con foco interno.

## 9. Estado de T1 (2026-11-03)

Hecho y comprobado en QEMU con `scripts/wm-probe.py`:

- Tira de pestañas en la barra de título (2 o más paneles), botón «+» siempre visible, × por pestaña, línea de acento y pestaña activa fundida con el cuerpo; contador de pestañas en la lista de ventanas de la barra superior.
- Clic en pestaña (activar), × (cerrar), «+» (nueva pestaña de la misma aplicación); cerrar la cabecera cede la ventana a otra pestaña.
- Atajos: **Ctrl+Mayús+T** nueva pestaña, **Ctrl+Tab / Ctrl+Mayús+Tab** siguiente/anterior, **Alt+1…9** ir a la pestaña n, **Alt+Q** cierra la pestaña, **Alt+Mayús+Q** cierra la ventana entera.
- «Misma aplicación»: terminal (otro PTY) y clientes lanzados por el WM (`soso-web`, `soso-files`; se anota `tab_app` y el lanzamiento pendiente se une a la ventana al llegar su `CREAR`).
- Un panel inactivo no se pinta ni recibe ratón/teclado; los terminales y clientes de todos los paneles se redimensionan con la ventana.

Notas:

- **Bug de kernel corregido de paso**: `wake_tty_waiters` (`kernel/src/task/pty.rs`) pasaba al lector de un PTY a `Runnable` sin copiarle los bytes ni poner `rax`; `sosh` en una terminal del WM leía EOF con la primera tecla y salía. Ahora sólo avisa al planificador, que es quien copia.
- Con teclado simulado por el monitor de QEMU, tras `ctrl-shift-t` el Mayús queda «pegado» y `alt-1` llega como `!` (33); con teclado real llega como 49. Alt+Mayús+letra llega en mayúscula (81), no en minúscula.
- `soso-files` termina con `sal panic: bytes: out of bounds` al arrancar en este QEMU (también sin tocar `fuente.sal`); queda por investigar, no es de las pestañas.
- Pendiente de T1: prueba guest `soso-wm --prueba-pestanas`; indicador de actividad (T3); pestaña «+» deshabilitada para clientes sin aplicación conocida (hoy no hace nada).

## 10. Estado final de esta ronda (2026-11-03)

**B1** (hecha). Todo el cromo de ventana —marco, sombra, línea de acento, tira de pestañas, iconos, «+», botones y glifos, barra de inserción— se describe como lista de primitivas (`escena.sal`) y la ejecuta el backend CPU; el botón de la barra, el menú de aplicaciones, la lista de ventanas y el menú contextual usan `wm_esc_rr`. Los degradados de fondo de la barra siguen por filas. Comprobado **píxel a píxel** contra capturas anteriores al refactor (0 píxeles de diferencia: una ventana, 3 pestañas con hover, menú abierto). Layout `GPRIM` (GSCEN v2, comando aparte: no rompe `GSCEN` v1) en `crates/soso-wm-proto/src/gprim.rs` y `comp_gprim_layout.sal`, con tests en Rust y Sal.

**T2** (hecha). Arrastrar pestaña dentro de la tira reordena; sacarla de la barra la despega a ventana propia (que sigue al puntero) y soltarla sobre la barra de otra ventana la integra (también ventanas enteras arrastradas por el título; el destino se marca con una barra de acento). Atajos: **Alt+Mayús+D** sacar la pestaña, **Alt+Mayús+H / L** moverla a izquierda/derecha (no hay teclas de cursor en el WM, de ahí H/L). Prueba guest en `soso-wm --prueba-layout` (fase `prueba_layout_pestanas`, verde en `cargo xtask test`).

**T3** (hecha en protocolo y WM). `HOLA` lleva `app_id` en el offset 8 (1 web, 2 archivos; 0 = ninguna, idéntico al HOLA de siempre); nueva op WM→cliente `VISIBLE` (25), enviada cuando un panel deja de verse (otra pestaña activa o ventana minimizada) o vuelve (`Evento::Visible` en `libsoso`; `soso-web` repinta al volver a verse). Punto de actividad en las pestañas inactivas de terminal con salida nueva (se apaga al activarlas). No probado de extremo a extremo con un cliente real que ignore `VISIBLE`: los clientes actuales lo toleran.

**T4** (parcial). Hecho: iconos de 10 px por tipo (terminal, archivos, web), ancho mínimo de pestaña 28 px, menú contextual con clic derecho (duplicar, sacar, cerrar otras, cerrar a la derecha, cerrar), claro/oscuro por los tokens de `tema.sal`. **No hecho**: animaciones (necesitan el reloj de fotogramas y repintado parcial; ver G3), flechas/menú de desbordamiento, «+» deshabilitado para clientes sin aplicación conocida.

**G1, G2, G4** (algoritmo validado, integración pendiente). `tools/gpu-prim/gprim.cu` ejecuta la lista de primitivas en GPU, un hilo por píxel y en orden, con las cinco clases: sólido (alfa), degradado, rectángulo redondeado (radios por esquina, borde), anillo (sombras) y **rejilla de celdas de texto** (atlas = bytes de la fuente PSF2; sirve para títulos y menús (G2) y para el terminal (G4)). `python3 tools/gpu-prim/validar.py` compila el oráculo Sal (`tools/gpu-prim/oraculo.sal`), compila el kernel con NVRTC, lo ejecuta en la GPU del equipo (RTX 5070 Ti) y compara píxel a píxel: **0 píxeles distintos** en 128×96 con 15 primitivas. Lo que **falta** para que soso lo use: empaquetar el kernel por arquitectura (SASS sm_86/sm_120 con ptxas/Docker), el manejador `GPRIM` en `kernel/src/drivers/gpu.rs` (subir celdas y fuente, lanzar el kernel por el canal de cómputo sobre el RT de VRAM y presentar), y que `soso-comp` la use cuando `gfx3d` esté listo. Eso sólo se puede cerrar en la placa.

**G3, G5, T7**: sin empezar. G3 (animaciones/miniaturas en GPU) depende de lo anterior; G5 (blur, cursor hardware) y T7 (`soso-web` con pestañas del WM) son opcionales.
