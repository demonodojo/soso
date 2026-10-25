# Plan escritorio: `soso-wm` en Sal

Gestor de ventanas gráfico en Sal con terminales sobre PTY, clientes vía TCP
`127.0.0.1:7430` y superficies en memoria compartida.

## Fases

| Fase | Estado | Contenido |
|------|--------|-----------|
| W1–W3 | Hecho | Dueño FB, entrada exclusiva, `SYS_FB_PRESENT_RECT`, PTY, SHM |
| S0 | Hecho | Operadores Int, tipo `Bytes`, builtin `syscall` (repo `../sal`) |
| S1 | Hecho | `user/sal/escritorio/`: `soso.sal`, superficie, utf8, fuente PSF2 |
| S2 | En curso | `term.sal` / `utf8.sal`: rejilla y UTF-8 básicos; CSI parcial |
| S3 | En curso | `ventanas.sal`, `tabla.sal` (16 ranuras), `proto.sal` |
| S4 | En curso | `soso-wm.sal`: bucle no recursivo, multi-cliente vía tabla, `--ejecutar` / `--prueba` / `--prueba-web` |
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

- Host: `cargo xtask test-sal` (incluye `tests/sal/escritorio/probar_host.py`)
- Guest: `init test` (pty + shm), `soso-wm --prueba` en `cargo xtask test`
- Visual opcional: `cargo xtask fb-shot`
