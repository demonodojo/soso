# T01 — Capturar una base reproducible sin alterar el checkout

**Hito:** SI-0 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** completada (15 de septiembre de 2026; resumen en [seguimiento/T01.md](seguimiento/T01.md), evidencia en `target/self-improvement/tasks/T01/resultado.md`).

**Dependencias:** ninguna.

## Objetivo y entrega

Manifiesto y copia reconstruible; ejecutar las suites base de C6 sobre la copia y registrar resultados, incluidos fallos previos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [rust-toolchain.toml](../../rust-toolchain.toml)
- [xtask/src/check.rs](../../xtask/src/check.rs)

## Archivos que se pueden cambiar

Entrega existente: `crates/soso-improve-core` (lógica portable, `no_std + alloc`), `tools/soso-improve` (binario host) y `user/soso-improve` (binario guest), con sus pruebas en `tools/soso-improve/tests/base.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** la primera versión se implementó en Python (15 de septiembre de
> 2026) y se portó a Rust el 16, porque el objetivo del plan es que todo pueda
> correr **dentro de soso**. Ver [seguimiento/T01.md](seguimiento/T01.md).

## Pasos

1. Implementar `soso-improve capturar --repo <ruta> --out <ruta>`. Exigir destino nuevo situado fuera de los archivos fuente; escribir `baseline.json` y el contenido de cada archivo de la base en un almacén por hash. **Sin parches de git**: reconstruir no puede depender de `clone` ni `apply`, que dentro de soso no existen.
2. Registrar inventario y hashes como identidad primaria, toolchain, OpenCode y herramientas de build. Si hay Git, adjuntar HEAD y estado staged/unstaged como metadatos opcionales; capturar un árbol sin .git debe funcionar. Versionar solo una lista explícita de variables SOSO pertinentes; nunca volcar todo el entorno.
3. Conservar contenido de archivos nuevos seleccionados como fuentes; para otros, registrar ruta/hash y exclusión. No recorrer discos de modelos, firmware ni `target/` indiscriminadamente. La captura debe permitir reconstruir exactamente la base declarada.
4. Detectar cambios del checkout durante la captura repitiendo estado y hashes; abortar como captura inestable si difieren. No hacer stash, reset, add ni commit.
5. Dar a cada comprobación un log y código de salida. El comando registra disponibilidad; las suites base se ejecutan como paso explícito posterior para no ocultar un build largo.

## Comprobación

`cargo test -p soso-improve-core -p soso-improve`. Fixtures: árbol limpio, repositorio con cambios preparados y sin preparar sobre un mismo archivo, binario, ruta con espacios, archivo nuevo, archivo ignorado y modificación concurrente. Comparar el estado del árbol antes/después; reconstruir la captura en temporal y comparar hashes.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Si la copia no reproduce los archivos, corregir la captura antes de editar el runtime. La falta de hardware se registra por separado.

Entregar `target/self-improvement/tasks/T01/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Cierre histórico conservado. La captura por contenido y los objetos permiten reconstruir sin Git; metadatos Git opcionales. Falta acreditar errores CLI y pruebas guest de T45/T49.

Validación nativa: **pendiente**. Condiciones adicionales: [T45](T45-cli-capacidades.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
