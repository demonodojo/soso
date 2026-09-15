# T29 — Ejecutar una campaña reproducible de diez tareas

**Hito:** SI-4 · **Tipo:** Integración y evaluación · **Estado:** pendiente.  
**Dependencias:** [T22](T22-primera-mejora.md), [T27](T27-reanudacion.md), [T28](T28-conocimiento.md)

## Objetivo y entrega

Campaña repetible con éxito o no-go publicado; SI-4 solo se cierra si alcanza los umbrales.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)

## Archivos que se pueden cambiar

Crear `tests/self-improvement/campaign.json`, implementar resumen en `tools/soso-improve/src/report.rs` con `tests/report.rs` y publicar `docs/self-improvement/campana.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Fijar diez tareas antes de ejecutar: completar las cinco reales T02 con otras cinco especificadas y comprobables, sin cambiar la selección tras ver resultados.
2. Ejecutar secuencialmente desde base aceptada; conservar intentos fallidos y bloqueados. Mantener servidor guest estable y candidato separado.
3. Introducir un fallo controlado del proceso agente y un reinicio del coordinador en intentos de prueba; comprobar recuperación T27.
4. Calcular aceptación, intentos, tiempo y tokens por mejora aceptada, distinguiendo errores de infraestructura, protocolo y programación.
5. Comprobar la base final con C6. Si cambia el modelo o runtime durante la campaña, registrar nueva versión y reevaluar banco reservado antes de usarla.

## Comprobación

`cargo test -p soso-improve --test report`; campaña real con criterio de SI-4: ≥5 mejoras aceptadas de 10, sin regresión y recuperación acreditada.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No rellenar mejoras con cambios triviales ajenos a las tareas fijadas ni quitar fallos del denominador.

Entregar `target/self-improvement/tasks/T29/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

