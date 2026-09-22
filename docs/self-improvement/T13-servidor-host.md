# T13 — Conectar el mismo runtime a un servidor de desarrollo en host

**Hito:** SI-1 / SI-2 · **Tipo:** Implementación host · **Estado:** completada (2026-09-22).

Resumen: [seguimiento/T13.md](seguimiento/T13.md).

**Dependencias:** [T09](T09-cancelacion-runtime.md), [T11](T11-http.md), [T12](T12-respuestas-sse.md)

## Objetivo y entrega

Servidor reutiliza pesos, devuelve JSON/SSE y maneja desconexión sin fuga persistente; evidencia host claramente identificada.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C4**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/examples/hostrun.rs](../../crates/soso-llm-core/examples/hostrun.rs)
- [crates/soso-llm-core/src/source.rs](../../crates/soso-llm-core/src/source.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-api/examples/serve_host.rs` y `tests/host_service.rs`; ajustar solo dependencias host necesarias.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Crear `serve_host --model-dir <ruta> --profile <json> --port <n>` como ejemplo de desarrollo. Token desde entorno según C3; cargar manifiesto/tokenizer/index verificados.
2. Reutilizar Runtime y las abstracciones de TensorSource. El mapper de hostrun fuga memoria intencionadamente por ser proceso corto: sustituirlo en este servidor por ownership y liberación reales, no copiar sus leaks.
3. Atender los tres endpoints C3 con T10–T12. Una sesión residente y contexto reconstruido en cada petición.
4. Integrar observador cancelable y comprobar escritura fallida. Registrar backend=host-cpu para distinguir esta evidencia de la ejecución guest.
5. Añadir un backend falso solo inyectable en tests, con errores y delays deterministas; el modo real debe requerir pesos explícitos.

## Comprobación

`cargo test -p soso-llm-api --features std --test host_service`. Después, ejecutar el ejemplo con pesos T03 y dos conversaciones distintas. Todos los tests automáticos usan sockets loopback y backend controlado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Esto no cierra inferencia dentro de soso. Si falla calidad, conservar salida para T14.

Entregar `target/self-improvement/tasks/T13/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Este ejecutable std es un arnés de desarrollo. Compartir codec/runtime con T16; su proceso host desaparece del circuito final. La evidencia host no verifica el servicio guest.

Validación nativa: **no aplicable a esta entrega de laboratorio/especificación**. Condiciones adicionales: [T16](T16-servicio-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
