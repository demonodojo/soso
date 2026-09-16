# T38 — Fijar revisiones y dependencias de la toolchain nativa

**Hito:** SI-7 · **Tipo:** Inspección acotada · **Estado:** pendiente.

**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

Inventario reproducible para T39–T42; ninguna herramienta marcada nativa solo por su nombre o --version.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [scripts/soso-rust-bootstrap.sh](../../scripts/soso-rust-bootstrap.sh)
- [config/rust-soso/README.md](../../config/rust-soso/README.md)
- [targets/x86_64-unknown-soso.json](../../targets/x86_64-unknown-soso.json)
- [user/soso-rustc/src/main.rs](../../user/soso-rustc/src/main.rs)
- [tools/wild-soso/src/main.rs](../../tools/wild-soso/src/main.rs)
- [tools/sosoas/src/main.rs](../../tools/sosoas/src/main.rs)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/toolchain-lock.json` y `toolchain-deps.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Inventariar lo real frente al stub: rustc, Cargo, LLVM/codegen, libstd/PAL, linker, ensamblador, proc macros, build scripts, C, empaquetadores y Git.
2. Fijar commits/versiones compatibles y hashes; identificar clones externos existentes antes de preparar otros. Registrar host/build/target de cada herramienta.
3. Comprobar que el bootstrap actual configura host Linux: compilar --target soso no genera automáticamente un rustc que ejecute en soso.
4. Definir sondas de resultado: emitir objeto, enlazar ejecutable, build offline con Cargo, macro procedural/build.rs, C+asm y creación de imagen.
5. Elegir perfil mínimo inicial y listar aparte dependencias del perfil lxdde completo. Estimar recursos usando builds medidos, no valores inventados.

## Comprobación

Cada herramienta tiene origen/revisión, plataforma de ejecución y prueba de salida. Comparar matriz con los comandos de SELF-HOSTING y el código de stubs.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No arrancar un build enorme de stage2 dentro de esta inspección ni sobrescribir un vendor compartido.

Entregar `target/self-improvement/tasks/T38/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Inventariar también x.py, scripts shell, generadores Python, build.rs, proc macros, C/asm, linker, empaquetado y gestores de paquetes. Asignar sustitución nativa o ficha C-xxx a cada dependencia transitiva.

Validación nativa: **no aplicable a esta entrega de laboratorio/especificación**. Condiciones adicionales: [T40](T40-compilador-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
