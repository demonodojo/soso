# T12 — Emitir respuestas completas y eventos SSE

**Hito:** SI-2 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.  
**Dependencias:** [T07](T07-parse-herramientas.md), [T08](T08-resultado-generacion.md), [T10](T10-api-json.md)

## Objetivo y entrega

Cliente de prueba consume ambas variantes sin pérdida; usage coincide con GenerationReport.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2–C3**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [crates/soso-llm-core/src/tokenizer.rs](../../crates/soso-llm-core/src/tokenizer.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-api/src/response.rs`, `src/sse.rs` y `tests/response.rs`; exponer módulos.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Definir una respuesta normalizada común a JSON completo y SSE: request id, model, contenido/llamada, uso y motivo de fin.
2. Emitir chat.completion o chat.completion.chunk según modo, índice 0 y rol assistant inicial. ToolCall se adapta a function.name/arguments e id.
3. Usar serializador JSON para todos los fragmentos; conservar Unicode y escapes. Enviar stop/length/tool_calls según el resultado, nunca inferir stop porque el socket cerró.
4. Proporcionar include_usage: evento de usage con choices vacío antes de DONE. Cancelación o fallo usa el canal de error C3.
5. Retener la llamada hasta validarla en T07; el primer emisor puede mandarla en un delta único. Probar reconstrucción desde varios deltas con fixtures.

## Comprobación

`cargo test -p soso-llm-api --features std --test response`. Reconstruir texto y argumentos de SSE y compararlos con JSON no streaming. Probar content null, comillas, Unicode, límite, error y exactamente un DONE en éxito.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No medir tokens re-tokenizando el texto de salida ni contar eventos SSE como tokens.

Entregar `target/self-improvement/tasks/T12/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

