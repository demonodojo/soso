# T06 — Renderizar la familia elegida con historial completo

**Hito:** SI-1 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.

**Dependencias:** [T03](T03-perfil-modelo.md), [T05](T05-validacion-chat.md)

## Objetivo y entrega

Tokens iguales a la referencia para los casos fijados y ausencia de duplicación BOS o pérdida de roles.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C2**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/src/chat.rs](../../crates/soso-llm-core/src/chat.rs)
- [crates/soso-llm-core/src/tokenizer.rs](../../crates/soso-llm-core/src/tokenizer.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-core/src/conversation/render.rs` y `tests/conversation_render.rs`; conectar el módulo de T04.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar `render_messages(input, profile, tokenizer) -> Result<Vec<u32>, ChatError>` siguiendo exactamente los fixtures T03.
2. Insertar system, historial, esquemas y resultados con delimitadores de la familia. Añadir un único prefijo de generación assistant.
3. Aplicar BOS y tokens especiales una sola vez donde corresponda. No serializar un token especial como sus caracteres ordinarios.
4. Contar la secuencia completa y rechazar ausencia de vocabulario o familia no implementada. Mantener la función antigua de pregunta única.
5. Para JSON dentro de la plantilla usar serializador, no concatenación manual de comillas de argumentos.

## Comprobación

`cargo test -p soso-llm-core --features std --test conversation_render`. Igualdad exacta con los cinco fixtures T03, Unicode, comillas y saltos; regresiones existentes de chat.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si el tokenizer actual no reproduce la referencia, completar la ficha correctiva de T03; no debilitar la igualdad a una comparación aproximada.

Entregar `target/self-improvement/tasks/T06/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
