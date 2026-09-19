# T09 — Añadir cancelación cooperativa a prefill y decode

**Hito:** SI-2 · **Tipo:** Implementación Rust de inferencia · **Estado:** completada (2026-09-19).

**Dependencias:** [T08](T08-resultado-generacion.md)

## Objetivo y entrega

Cancelación comprobada en prompt largo y decode; la siguiente petición funciona y los adaptadores existentes pasan.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2–C4**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/src/runtime.rs](../../crates/soso-llm-core/src/runtime.rs)
- [crates/soso-llm-core/src/parallel.rs](../../crates/soso-llm-core/src/parallel.rs)

## Archivos que se pueden cambiar

Editar runtime y `generation.rs` de T08; crear `crates/soso-llm-core/tests/generation_cancel.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Añadir GenerationObserver con recepción de tokens y checkpoint cancelable. Proporcionar observador por defecto para los adaptadores antiguos.
2. Comprobar cancelación antes de prefill, entre tokens de prefill, entre capas y durante decode/drafts. Devolver StopReason::Cancelled.
3. Asegurar que no se genera ni publica otro token después del punto que acepta cancelar. No recurrir a punteros globales nuevos hacia sesiones movibles.
4. Probar reutilización del mismo Runtime después de cancelar: reset de secuencia antes de la siguiente petición, sin contaminar tokens ni estado.
5. Conservar hooks de diagnóstico existentes; documentar el tramo máximo no interrumpible en lugar de prometer latencia constante.

## Comprobación

`cargo test -p soso-llm-core --features std --test generation_cancel`. Cancelar en cada fase con contador determinista, inyectar error en observador y comparar la siguiente generación con Runtime nuevo.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Las llamadas de red y liberación de workers guest pertenecen a T17; no introducir syscalls en core.

Entregar `target/self-improvement/tasks/T09/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
