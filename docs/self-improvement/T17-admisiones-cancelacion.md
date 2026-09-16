# T17 — Atender ocupado, health y desconexión durante inferencia

**Hito:** SI-2 · **Tipo:** Implementación guest acotada · **Estado:** pendiente.

**Dependencias:** [T09](T09-cancelacion-runtime.md), [T16](T16-servicio-guest.md)

## Objetivo y entrega

Servicio no queda ocupado para siempre ni mezcla conversaciones tras cancelar; límites de conexiones y memoria comprobados.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3–C4**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [user/soso-llm/src/net.rs](../../user/soso-llm/src/net.rs)
- [user/libsoso/src/sys.rs](../../user/libsoso/src/sys.rs)
- [kernel/src/net/mod.rs](../../kernel/src/net/mod.rs)

## Archivos que se pueden cambiar

Crear `user/soso-llm/src/serve_poll.rs`; conectar en serve.rs. Extraer estado puro comprobable a `crates/soso-llm-api/src/admission.rs` con `tests/admission.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar un estado con una generación activa y cero cola; máximo dos conexiones HTTP auxiliares parciales, con buffers/límites y deadline propios.
2. Desde checkpoints de generación, hacer un sondeo breve: aceptar, leer una cantidad acotada y responder health u ocupado. No bloquear esperando una cabecera completa ni invocar generación recursiva.
3. Distinguir EAGAIN, EOF y error con syscalls reales. Probar cómo detecta cierre el fd activo sin consumir datos de otra petición.
4. Conectar desconexión/deadline al observador cancelable. Mantener heartbeat SSE como comentario de transporte si hace falta; no contarlo como token.
5. Restablecer admisión, KV por petición, hooks y workers tras salida normal, error o cancelación.

## Comprobación

`cargo test -p soso-llm-api --features std --test admission`; build guest. En T19 probar cliente lento, segunda generación=429, health durante prefill y cierre activo. Medir latencia real hasta siguiente checkpoint.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si la syscall no permite el sondeo supuesto, adjuntar reproducción mínima y especificar una ficha ABI; no usar un bucle bloqueante oculto.

Entregar `target/self-improvement/tasks/T17/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
