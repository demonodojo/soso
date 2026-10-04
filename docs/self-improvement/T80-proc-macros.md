# T80 — Proc macros y dylib en el guest

**Hito:** SI-7 · **Tipo:** Integración condicionada · **Estado:** **hecha** (2026-10-03). Resumen en `target/self-improvement/tasks/T80/resultado.md` y [seguimiento/T80.md](seguimiento/T80.md).
**Dependencias:** [T41](T41-cargo-offline.md).
**Origen:** cierre de T41 («Si falta dlopen/proc macros, crear ficha específica; no
preexpandir en Linux y contabilizar la campaña como completamente nativa»).

## Problema

Cargo y rustc corren en el guest (T40, T41), pero el target `x86_64-unknown-soso`
no tiene `dlopen`: rustc responde `dropping unsupported crate type proc-macro` y
cualquier crate que dependa de un proc macro (`serde_derive`, `thiserror`…) no
compila. Sin ellos, el perfil de automejora no cubre la mayoría de los crates
reales; preexpandirlos en Linux invalidaría la cobertura nativa.

## Qué hay que decidir

Dos rutas, sin elegir todavía:

1. **Cargar el proc macro como biblioteca dinámica**: un `dlopen` mínimo para
   ELF `ET_DYN` (o el equivalente de rustc, `libloading`), con reubicaciones y
   TLS. Exige un cargador en el PAL y que `wild-soso` produzca `cdylib`.
2. **Enlazar los proc macros dentro del propio rustc** (como harían `-Zproc-macro-
   execution-strategy=cross-thread` con el ejecutable ya cargado): el cargo
   compila el proc macro como `bin`, rustc lo lanza como proceso y habla con él
   por tuberías.

La ruta 2 reaprovecha las tuberías y `spawn` con entorno de T41 (C-101, C-111).

## Alcance

Una C-xxx por hueco real, con reproducción en el guest. Fixture previsto:
`tests/self-improvement/native/cargo/ws4` con un `proc-macro` derive trivial.

## Comprobación

Dentro del guest: `cargo build --offline` de un workspace con un crate
`proc-macro` y un consumidor; el consumidor imprime algo que sólo el macro puede
haber generado.

## Cierre

- [x] Ruta decidida: objetos compartidos propios (ver C-121…C-125). Pendiente de pasar a [DECISIONES.md](DECISIONES.md).
- [x] Fixture (`tests/self-improvement/native/procmacro/`) y `zerocopy-derive` (mkfs-soso) compilados y ejecutados en el guest, con evidencia.
- [x] Límites explícitos (`resultado.md`).
