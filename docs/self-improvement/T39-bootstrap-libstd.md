# T39 — Hacer reproducible el bootstrap de libstd para soso

**Hito:** SI-7 · **Tipo:** Implementación de build · **Estado:** pendiente.

**Dependencias:** [T38](T38-toolchain-inventario.md), [T50](T50-cambios-contenido.md)

## Objetivo y entrega

La misma revisión produce sysroot utilizable; segunda preparación no duplica parches. Esto acredita libstd target, no rustc host soso.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [scripts/soso-rust-bootstrap.sh](../../scripts/soso-rust-bootstrap.sh)
- [config/rust-soso/apply-patches.sh](../../config/rust-soso/apply-patches.sh)
- [config/rust-soso/config.toml](../../config/rust-soso/config.toml)
- [config/rust-soso/sys/pal/soso/mod.rs](../../config/rust-soso/sys/pal/soso/mod.rs)

## Archivos que se pueden cambiar

Mantener bootstrap/apply-patches como entrada de desarrollo; extraer preparación, verificación de hashes y receta a un módulo Rust portable, invocable desde tools/soso-improve y user/soso-improve. Añadir pruebas con vendor temporal. La ejecución nativa de build consume T40–T42.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** esta ficha pedía Python; se reescribió el 16 de septiembre de 2026
> a Rust, porque el objetivo del plan es que todo pueda correr **dentro de soso**
> y soso no tiene intérprete de Python. Ver
> [seguimiento/T01.md](seguimiento/T01.md).

## Pasos

1. Exigir revisión del lock T38 y destino externo específico; comprobar inventario/hashes y, opcionalmente, HEAD antes de aplicar cambios portables. No resetear un vendor con cambios.
2. Separar preparación y build en receta declarativa: argv, cwd, inputs/outputs y hashes por paso. Hacer preparación idempotente y reportar cada fallo. Inventariar helpers de los scripts y asignar sustitución nativa; mover solo sus tests a Rust no basta.
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

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Separar semilla cruzada de reconstrucción nativa. Los scripts existentes sirven de referencia de desarrollo; entregar receta declarativa ejecutable por Rust y hashes de la semilla. T40–T42 prueban reconstrucción sin x.py, Bash ni Python.

Validación nativa: **pendiente**. Condiciones adicionales: [T40](T40-compilador-nativo.md), [T41](T41-cargo-offline.md), [T42](T42-c-link-imagen.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
