# T20 — Configurar OpenCode para el proveedor soso

**Hito:** SI-3 · **Tipo:** Configuración · **Estado:** pendiente.  
**Dependencias:** [T14](T14-evaluacion-modelo.md), [T19](T19-qemu-e2e.md)

## Objetivo y entrega

Configuración reproducible con solo proveedor previsto para la ejecución y límites medidos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3, C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)

## Archivos que se pueden cambiar

Crear `opencode.json` y `.opencode/agents/soso-improve.md`; documentar configuración efectiva en `docs/self-improvement/opencode.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Fijar versión instalada y revisión/config schema correspondiente; revalidar documentación oficial. Crear proveedor @ai-sdk/openai-compatible con baseURL y token por entorno.
2. Usar límites calibrados T14 y fijar model/small_model a soso/soso-coder. Auditar título, resumen, compactación y modelos configurados en agentes heredados.
3. Configurar agente con lectura/búsqueda/edición/pruebas, trabajo delegado desactivado y alcance del checkout. El prompt manda seguir TASK.md y reportar evidencia.
4. Generar configuración efectiva en un entorno de configuración aislado, evitando incorporar credenciales o plugins globales. No activar autoaprobación universal para resolver la automatización.
5. Definir timeouts a partir de la medición; comandos permitidos de pruebas acordes con la ficha. Registrar hash de configuración y versión del proveedor.

## Comprobación

Validar JSON contra esquema fijado y consultar configuración/modelos efectivos sin iniciar una tarea. T21 comprobará tráfico y herramientas; esta ficha no certifica aún inferencia con OpenCode.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si el esquema/versionado cambió, actualizar la configuración con evidencia oficial; no asumir que el ejemplo del plan es operativo sin verificación.

Entregar `target/self-improvement/tasks/T20/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

