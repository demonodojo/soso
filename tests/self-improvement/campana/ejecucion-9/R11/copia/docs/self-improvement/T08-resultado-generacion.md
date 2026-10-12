# T08 — Devolver motivo de parada y consumo real del runtime

**Hito:** SI-1 / SI-2 · **Tipo:** Implementación Rust de inferencia · **Estado:** pendiente.  
**Dependencias:** Ninguna; puede iniciarse ahora.

## Objetivo y entrega

Estadísticas y motivo coinciden con generación observada; los usuarios actuales de Runtime conservan su salida.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [crates/soso-llm-core/src/runtime.rs](../../crates/soso-llm-core/src/runtime.rs)
- [crates/soso-llm-core/src/sample.rs](../../crates/soso-llm-core/src/sample.rs)
- [crates/soso-llm-core/tests/arch_ext.rs](../../crates/soso-llm-core/tests/arch_ext.rs)

## Archivos que se pueden cambiar

Editar `crates/soso-llm-core/src/runtime.rs`; crear `src/generation.rs` y `tests/generation_report.rs` en la crate; exponer en `lib.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Añadir GenerationReport y StopReason según C2 y una variante nueva de generación que devuelva ese informe.
2. Inspeccionar los caminos normal, paralelo, planned y drafts. Contar cada token aceptado exactamente una vez, incluyendo un stop consumido según C2.
3. Admitir el conjunto de stop IDs del perfil; detener antes de decodificar esos delimitadores. Diferenciar límite de salida/contexto y error de inferencia.
4. Conservar firmas actuales mediante adaptadores y su contrato de tokens prompt+salida. No copiar el bucle completo en un segundo motor que diverja.
5. No cambiar sampling, matvec, KV eviction ni políticas de memoria como parte de esta tarea.

## Comprobación

`cargo test -p soso-llm-core --features std --test generation_report`, después suite core. Fixture pequeño determinista: stop primero, stop después de texto, max=1, máximo de contexto y draft aceptado. Comparar tokens antiguos/nuevos.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si el refactor exige cambios numéricos, separarlos; esta tarea solo cambia control e informe.

Entregar `target/self-improvement/tasks/T08/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

