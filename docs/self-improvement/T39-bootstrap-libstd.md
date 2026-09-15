# T39 — Hacer reproducible el bootstrap de libstd para soso

**Hito:** SI-7 · **Tipo:** Implementación de build · **Estado:** pendiente.  
**Dependencias:** [T38](T38-toolchain-inventario.md)

## Objetivo y entrega

La misma revisión produce sysroot utilizable; segunda preparación no duplica parches. Esto acredita libstd target, no rustc host soso.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [scripts/soso-rust-bootstrap.sh](../../scripts/soso-rust-bootstrap.sh)
- [config/rust-soso/apply-patches.sh](../../config/rust-soso/apply-patches.sh)
- [config/rust-soso/config.toml](../../config/rust-soso/config.toml)
- [config/rust-soso/sys/pal/soso/mod.rs](../../config/rust-soso/sys/pal/soso/mod.rs)

## Archivos que se pueden cambiar

Editar bootstrap/apply-patches y documentación asociada; añadir las pruebas del bootstrap a `tools/soso-improve` (o a `xtask`, si encaja mejor con el flujo de build) con vendor temporal.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** esta ficha pedía Python; se reescribió el 16 de septiembre de 2026
> a Rust, porque el objetivo del plan es que todo pueda correr **dentro de soso**
> y soso no tiene intérprete de Python. Ver
> [seguimiento/T01.md](seguimiento/T01.md).

## Pasos

1. Exigir revisión del lock T38 y destino externo específico; comprobar HEAD y estado antes de aplicar parches. No resetear un vendor con cambios.
2. Separar preparación y build; hacer preparación idempotente y reportar cada fallo sin tragarse errores con éxito posterior.
3. Corregir rutas de salida reales de los wrappers/linker y producir manifiesto de sysroot con target y hashes.
4. Compilar libstd cruzada para el target soso y ejecutar un programa mínimo enlazado con esa libstd dentro del guest.
5. Si la PAL falla por un símbolo/semántica, generar una ficha C-xxx según T40 y cerrarla antes de afirmar bootstrap completo.

## Comprobación

`cargo test -p soso-improve` para revisión incorrecta, preparación doble y vendor sucio. Después build real `cargo xtask rust-build-std` y humo guest.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Las funciones stub de PAL no se validan porque libstd enlace; el programa debe ejecutarse y realizar su efecto.

Entregar `target/self-improvement/tasks/T39/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

