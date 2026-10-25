# Plan escritorio: `soso-wm` en Sal

Gestor de ventanas gráfico en Sal con terminales sobre PTY, clientes vía TCP
`127.0.0.1:7430` y superficies en memoria compartida.

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
| W7 | En curso | `init test` pty/shm; xtask usa `--prueba` sin `&` en sosh |
| WM+web | Hecho | Bucle WM (`wm.sal`), `demo-cliente`, `soso-web` en ventana SHM + E2E xtask |

## Atajos (`soso-wm`)

- Alt+Intro — terminal nueva  
- Alt+Tab — siguiente ventana  
- Alt+Q — cerrar ventana  
- Alt+T — flotante / mosaico  
- Alt+Mayús+E — salir a consola  
- Ctrl+Alt+Retroceso — mata al dueño de pantalla (kernel)

## Pruebas

- Host: `cargo xtask check` o `python3 tests/sal/escritorio/probar_host.py` (terminal, utf8, ventanas, proto, fuente)
- Guest: `init test` (pty + shm), `soso-wm --prueba` en `cargo xtask test`
- Visual opcional: `cargo xtask fb-shot`
