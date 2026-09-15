# T05 — Validar historial y esquemas de herramientas

**Hito:** SI-1 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.  
**Dependencias:** [T04](T04-dominio-chat.md)

## Objetivo y entrega

Cada entrada inválida devuelve un error comprobable sin ejecutar herramientas ni mutar el historial.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C3**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [crates/soso-llm-core/src/lib.rs](../../crates/soso-llm-core/src/lib.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-core/src/conversation/validate.rs` y `tests/conversation_validation.rs`; conectar desde `conversation.rs` creado en T04.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar `validate_input(&ChatInput)` con límites C3: roles coherentes, nombres únicos, IDs no duplicados y resultados que correspondan a llamadas pendientes.
2. Aceptar varias llamadas previas solo cuando cada resultado resuelva su ID; impedir generar con una llamada anterior pendiente de resultado.
3. Implementar validación acotada del subconjunto JSON Schema: object/properties/required/additionalProperties, array/items, tipos escalares, enum y anyOf. Limitar profundidad; rechazar referencias o keywords de validación no implementadas.
4. Distinguir anotaciones que no cambian validación —description/title— de reglas que nunca deben ignorarse. Registrar en error la ruta exacta al argumento inválido.
5. Validar tool_choice, existencia de herramientas y argumentos completos. No corregir silenciosamente JSON generado ni inventar resultados.

## Comprobación

`cargo test -p soso-llm-core --features std --test conversation_validation`. Positivos anidados y negativos: ID huérfano, resultado duplicado, llamada pendiente, tipo erróneo, requerido ausente, propiedad extra, esquema demasiado profundo y keyword no soportada.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Un esquema real de OpenCode fuera del subconjunto requiere fixture y ampliación acotada; no desactivar la validación para aceptarlo.

Entregar `target/self-improvement/tasks/T05/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

