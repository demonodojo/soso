# T07 — Extraer llamadas válidas de la salida del modelo

**Hito:** SI-1 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.

**Dependencias:** [T03](T03-perfil-modelo.md), [T05](T05-validacion-chat.md)

## Objetivo y entrega

El resultado final es idéntico para todas las particiones de la misma entrada; los errores no originan llamadas ejecutables.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2–C3**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/src/tokenizer.rs](../../crates/soso-llm-core/src/tokenizer.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-core/src/conversation/tools.rs` y `tests/conversation_tools.rs`; exportar desde el módulo T04.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar un parser incremental para el formato de herramienta fijado en T03: entrada por fragmentos y cierre explícito de generación.
2. Retener prefijos de delimitadores incompletos entre fragmentos. Separar texto ordinario y llamada sin perder bytes ni emitir delimitadores como contenido.
3. Al cerrar una llamada, parsear argumentos JSON completos y validar con T05 nombre/esquema/tool_choice; producir ToolCall con ID asignado por servidor.
4. Aplicar el límite de argumentos y una llamada emitida por turno. Texto que explica un comando o lo incluye entre backticks sigue siendo texto.
5. Una llamada truncada, varias llamadas nuevas o una llamada inválida produce error; nunca ejecutar ni publicar parcialmente esos argumentos como llamada completa.

## Comprobación

`cargo test -p soso-llm-core --features std --test conversation_tools`. Fragmentar cada fixture en todos los puntos de corte, incluyendo Unicode y delimitadores. Probar texto literal, JSON inválido, nombre desconocido, truncado y límite excedido.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si se desconoce el formato real, volver a la evidencia T03; no añadir detectores heurísticos de bloques de código.

Entregar `target/self-improvement/tasks/T07/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
