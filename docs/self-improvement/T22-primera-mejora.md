# T22 — Resolver una tarea real usando la inferencia guest

**Hito:** SI-3 · **Tipo:** Integración y evaluación · **Estado:** pendiente.

**Dependencias:** [T02](T02-banco.md), [T19](T19-qemu-e2e.md), [T21](T21-opencode-contrato.md)

## Objetivo y entrega

Un cambio útil generado por el modelo de soso y validado independientemente. Adjuntar base, parche y artefactos que permiten repetirlo.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Cambiar únicamente las rutas de una de las tareas reales de T02; guardar evidencia en `target/self-improvement/first-improvement/` y resumen en `docs/self-improvement/primera-mejora.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Seleccionar la tarea real más pequeña con verificador independiente. Preparar copia aislada, TASK.md y contexto mínimo; fijar base y modelo del servidor guest.
2. Ejecutar el mismo verificador sobre la base y registrar el fallo esperado o la medida de referencia. Mantener servidor de inferencia separado de la VM candidata.
3. Lanzar OpenCode con --format json y proveedor T20; dejarle leer, editar y ejecutar sus comprobaciones dentro del alcance.
4. Conservar la reparación de un error de herramienta/build si sucede; no introducir un fallo artificial en el repo principal para obtener esa evidencia.
5. Aplicar el parche a una copia de validación limpia, ejecutar criterio independiente, C6 y boot QEMU. Correlacionar requests OpenCode con logs de inferencia guest.

## Comprobación

Comprobador exacto T02 antes/después, `cargo xtask check` y `cargo xtask test` sobre el candidato. Revisar diff real y transcript de herramientas, no solo respuesta final.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si falla calidad o presupuesto, conservar intento y volver a T14/T21 según causa. No completar manualmente el parche y atribuírselo al modelo.

Entregar `target/self-improvement/tasks/T22/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T35](T35-opencode-nativo.md), [T41](T41-cargo-offline.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
