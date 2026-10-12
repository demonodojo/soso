# T03 — Fijar un modelo y obtener fixtures independientes de su chat

**Hito:** SI-0 / SI-1 · **Tipo:** Inspección y fixtures · **Estado:** pendiente.  
**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

Perfil candidato identificable y fixtures reproducibles; diferencias del tokenizer, si existen, convertidas en fichas concretas previas a T06.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C2**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [rootfs/etc/llm.conf](../../rootfs/etc/llm.conf)
- [xtask/src/live_models.rs](../../xtask/src/live_models.rs)
- [crates/gguf2som/src/lib.rs](../../crates/gguf2som/src/lib.rs)
- [crates/soso-llm-core/src/tokenizer.rs](../../crates/soso-llm-core/src/tokenizer.rs)

## Archivos que se pueden cambiar

Crear `tests/self-improvement/model-profile.schema.json`, fixtures pequeños en `tests/self-improvement/reference/` y `docs/self-improvement/modelo.md`. Pesos y herramientas externas permanecen fuera de Git.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Inventariar modelos disponibles localmente. Evaluar primero el candidato Qwen2.5-Coder de C1; si falta, registrar la ubicación de entrada necesaria. No seleccionar tiny sintético como candidato de programación.
2. Guardar `model-lock.json` en artefactos: origen/revisión, arquitectura, cuantización, hashes, familia, plantilla original, tokenizer original, manifest/index `.som` y parámetros de referencia.
3. Usar el tokenizer oficial de esa revisión para obtener texto renderizado y token IDs de cinco conversaciones: simple, system, historial, esquema de herramienta y resultado de herramienta. Guardar script/comando y versiones con los fixtures.
4. Comparar los IDs con el tokenizer `.som` actual. Identificar la primera divergencia y clasificarla: token especial, normalización, segmentación, conversión o plantilla.
5. Fijar el formato exacto de llamada de la familia, stops y tratamiento de errores. Publicar perfil candidato aunque la calidad siga pendiente; T14 decidirá si sirve para el agente.

## Comprobación

Regenerar los cinco fixtures dos veces y comparar hashes. Verificar todos los IDs contra vocab_size. La referencia debe provenir del tokenizer original, no del renderer que se va a implementar.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si faltan pesos/tokenizer originales, entregar inventario y entrada pendiente. No inventar IDs ni inferir soporte de herramientas a partir de ChatML.

Entregar `target/self-improvement/tasks/T03/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

