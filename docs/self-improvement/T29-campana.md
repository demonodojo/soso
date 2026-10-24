# T29 — Ejecutar una campaña reproducible de diez tareas

**Hito:** SI-4 · **Tipo:** Integración y evaluación · **Estado:** **hecha con GO mínimo** (2026-10-13). Con el 14B: 1→2→3 y 2 de 10 (NO-GO); con **Gemma 4 12B servido por soso**: **5 de 10** en el banco R11–R20, umbral de SI-4 cumplido ([campaña 6](campana.md), [seguimiento](seguimiento/T29.md)). Confirmado en un banco nuevo que nadie había visto (R21–R30, campaña 7): otros **5 de 10**; pero un tercer banco ciego (R31–R40, campañas 8 y 9) da **2 de 10** con el agente anterior y con el mejorado: el GO es frágil (12 de 30 en conjunto).

**Dependencias:** [T22](T22-primera-mejora.md), [T27](T27-reanudacion.md), [T28](T28-conocimiento.md), [T49](T49-pruebas-guest.md)

**Modelo de trabajo:** `qwen2.5-coder-14b` (decisión D9 de [DECISIONES.md](DECISIONES.md), 2026-10-11): GO 10/10 en T14 y único que resuelve R03 a T=0 con OpenCode. Lock: `target/self-improvement/tasks/T14/model-lock-14b.json`; el 7B queda como perfil ligero.

**Actualización (2026-10-23).** Tras las campañas 45–57 el banco R31–R40 da **2/10** con Gemma 4 12B, **5–7/10** con Qwen3.6-35B-A3B (MoE, llama.cpp) y **9/10** con el combo MoE → Qwen3.6-27B denso (R37 es el único hueco local); la calibración con Claude Sonnet 5.5 da **10/10** con el mismo arnés, así que el límite es el modelo. Modelo de trabajo: [D11](DECISIONES.md) (aceptada el 2026-10-24). Detalle: [campana.md](campana.md), [análisis de rechazos](resultados-campanas/analisis-rechazos-45-47.md) y [campaña 56](resultados-campanas/campana-56-combo-moe-denso). El banco R31–R40 **ya no es ciego**: para una cifra honesta hace falta uno nuevo.

## Objetivo y entrega

Campaña repetible con éxito o no-go publicado; SI-4 solo se cierra si alcanza los umbrales.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear `tests/self-improvement/campaign.json`, implementar resumen en `crates/soso-improve-core/src/report.rs` con pruebas compartidas y orden report en ambos frontends y publicar `docs/self-improvement/campana.md`.

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

`cargo test -p soso-improve-core --test report`; campaña real con criterio de SI-4: ≥5 mejoras aceptadas de 10, sin regresión y recuperación acreditada.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

No rellenar mejoras con cambios triviales ajenos a las tareas fijadas ni quitar fallos del denominador.

Entregar `target/self-improvement/tasks/T29/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T51](T51-aceptacion-circuito-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
