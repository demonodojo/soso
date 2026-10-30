# T84 — Biblioteca Sal `std/io` en el SDK

**Hito:** SI-6 · **Tipo:** Infraestructura · **Estado:** ver catálogo

**Dependencias:** T83

## Objetivo

Módulos Sal importables (`std/io`: ABI, ficheros, procesos, red, entorno, reloj) publicados en `rootfs/usr/share/sal/std/io` y resolubles vía `Sal.toml` `[dependencies]`, sin duplicar syscalls en cada programa.

## Entrega

- Fuentes en repo `sal`: `std/io/*.sal`
- Copia al SDK/rootfs en `cargo xtask sal-build`
- Prueba: `tests/sal/soso-lib.sal` compila para `--target soso`

## Comprobación

```sh
cargo xtask sal-build --cross-only
# compilar tests/sal/soso-lib.sal con sal del host
```
