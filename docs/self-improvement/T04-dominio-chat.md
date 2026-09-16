# T04 — Añadir tipos de conversación compartidos

**Hito:** SI-1 · **Tipo:** Implementación Rust host/no_std · **Estado:** completada (16 de septiembre de 2026; resumen en [seguimiento/T04.md](seguimiento/T04.md)).

**Dependencias:** ninguna.

## Objetivo y entrega

Tipos utilizables sin std y tests de conservación de información; métodos antiguos siguen compilando.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/src/lib.rs](../../crates/soso-llm-core/src/lib.rs)
- [crates/soso-llm-core/Cargo.toml](../../crates/soso-llm-core/Cargo.toml)
- [user/soso-hf/Cargo.toml](../../user/soso-hf/Cargo.toml)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-core/src/conversation.rs` y `tests/conversation_types.rs` dentro de la crate; editar su `lib.rs` y `Cargo.toml`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Declarar exactamente los tipos de dominio de C1 y errores diferenciados. Mantenerlos independientes de HTTP, sockets y syscalls.
2. Añadir serde/serde_json con alloc según C1. Exponer el módulo; no cambiar chat::render ni Runtime en esta ficha.
3. Representar explícitamente texto ausente frente a texto vacío; conservar argumentos como JSON serializado y esquemas como Value.
4. Derivar o implementar serialización del dominio solo para fixtures/persistencia. No dar por hecho que ese JSON interno es ya el formato Chat Completions.

## Comprobación

`cargo test -p soso-llm-core --features std --test conversation_types`; `cargo check -p soso-llm-core --no-default-features`. Casos: ida/vuelta de cada rol, null, Unicode, varias llamadas y resultados asociados.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Si una dependencia activa std en guest, corregir features antes de avanzar; no crear un segundo dominio dentro de la API.

Entregar `target/self-improvement/tasks/T04/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
