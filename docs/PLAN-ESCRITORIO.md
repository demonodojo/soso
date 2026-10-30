# Plan escritorio: `soso-wm` en Sal

Gestor de ventanas gráfico en Sal con terminales sobre PTY, clientes vía TCP
`127.0.0.1:7430` y superficies en memoria compartida. Skills Cursor/Claude:
**`soso-wm`** (WM) y **`soso-comp`** (compositor GPU `:7432`).

## Fases

| Fase | Estado | Contenido |
|------|--------|-----------|
| W1–W3 | Hecho | Dueño FB, entrada exclusiva, `SYS_FB_PRESENT_RECT`, PTY, SHM |
| S0 | Hecho | Operadores Int, tipo `Bytes`, builtin `syscall` (repo `../sal`) |
| S1 | Hecho | `user/sal/escritorio/`: `soso.sal`, superficie, utf8, fuente PSF2 |
| S2 | Hecho (host) | `term.sal`: scroll, historial, redimensionado; `utf8.sal` estricto; Spleen PSF2 en `/lib/fonts/`; pruebas en `tests/sal/escritorio/` vía `cargo xtask check` |
| S3 | Hecho (host) | Ventanas/mosaico sin empaquetado en API; `tabla.sal` acumulador 32 B; clics/arrastre en `wm.sal`; `proto.sal` + `cargo test -p soso-wm-proto` |
| S4 | Hecho (build) | WM: ≤8 PTYs, barra, ratón, mosaico Alt+T, acumulador cliente, `--prueba` imprime `soso-wm: prueba OK`; guest: `cargo xtask test` |
| W6 | Hecho | `crates/soso-wm-proto`, `libsoso::ventana`, `soso-web --grafico` (Danio bajo demanda en WM) |
| W7 | Hecho | `init test` pty/shm; xtask usa `--prueba` sin `&` en sosh |
| WM+web | Hecho | Bucle WM (`wm.sal`), `demo-cliente`, `soso-web` en ventana SHM + E2E xtask |
| W8 | Hecho | Navegador de lectura: motor CSS (flex/grid/float/position, @media), PNG, formularios, 4 pestañas, marcadores, cromo URL, Alt+W / `--prueba-web` |
| W9 | Hecho | Lanzador (`lanzador.sal`): menú Aplicaciones (Terminal, Web, Archivos, Salir), accesos directos Terminal/Web; cliente `soso-files` (`getdents`, SHM, DANIO) |
| W10 | Hecho | Tema oscuro/claro (`tema.sal`), barra y menú translúcidos, marcos redondeados, 5 fuentes Spleen (`fuentes.sal`); menú «Modo claro/oscuro» y **Alt+D**; **Ctrl+/-** escala app con foco, **Ctrl+Mayús+/-** escala WM; protocolo `op TEMA`/`op ESCALA`, offsets VENTANA 24/28 |

## Atajos (`soso-wm`)

- Alt+Intro — terminal nueva  
- Alt+Tab — siguiente ventana  
- Alt+Q — cerrar ventana  
- Alt+T — flotante / mosaico  
- Alt+W — abrir `soso-web --grafico --local /var/paquetes/soso-web/share/ejemplo/web-prueba.html`  
- Alt+D — alternar tema claro/oscuro (WM, terminal, clientes)  
- Ctrl+**+** / Ctrl+**-** — tamaño de letra de la ventana con foco (5 niveles; en teclado US usar Ctrl+**=** para subir la app)  
- Ctrl+Mayús+**+** / Ctrl+Mayús+**-** — tamaño de letra del WM (barra, menú, títulos)  
- Alt+Mayús+E — salir a consola  
- Ctrl+Alt+Retroceso — mata al dueño de pantalla (kernel)

## Pruebas

- Host: `cargo xtask check` o `python3 tests/sal/escritorio/probar_host.py` (terminal, utf8, ventanas, lanzador, files_path, files_dirent, proto, fuente)
- Guest: `init test` (pty + shm), `soso-wm --prueba` y `soso-wm --prueba-layout` en `cargo xtask test`
- Visual opcional: `cargo xtask fb-shot`
