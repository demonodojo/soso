# T16 — Servir HTTP en guest con el modelo residente

**Hito:** SI-2 · **Tipo:** Implementación guest · **Estado:** hecho (validación nativa / T14 go pendientes).

**Dependencias:** [T10](T10-api-json.md), [T11](T11-http.md), [T12](T12-respuestas-sse.md), [T14](T14-evaluacion-modelo.md), [T15](T15-sesion-residente.md)

## Objetivo y entrega

JSON completo y SSE salen del runtime guest, sin trazas de consola mezcladas y sin recargar pesos entre peticiones.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C4**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [user/soso-llm/src/main.rs](../../user/soso-llm/src/main.rs)
- [user/soso-llm/src/net.rs](../../user/soso-llm/src/net.rs)
- [user/libsoso/src/sys.rs](../../user/libsoso/src/sys.rs)

## Archivos que se pueden cambiar

Crear `user/soso-llm/src/serve.rs`; integrar en main.rs y Cargo.toml, usando session.rs de T15.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar el subcomando serve de C4. Validar argumentos, perfil/modelo y token-file antes de escuchar; mensajes de uso en español.
2. Poseer listeners ask y HTTP dentro del mismo proceso con una sesión. Si 7420 está ocupado por otro proceso, devolver error claro y no matar ni duplicar la sesión.
3. Conectar lectura HTTP acotada, autenticación Bearer, request JSON, render, generación y respuesta. HTTP no usa generar_tokens como emisor textual.
4. En modo secuencial atender reconexiones y errores de lectura/escritura. /health informa loading/ready/failed y modelo/backend efectivo; /models publica el alias fijado.
5. Documentar que ocupado/cancelación mientras se calcula se completan en T17; no declarar aún cierre del servicio de producción.

## Comprobación

Build guest en cwd `user/`; pruebas host de api. Humo loopback guest o conexión reenviada cuando exista T18: health, autenticación inválida, dos respuestas y modelo inexistente.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

No modificar kernel/syscalls para facilitar el primer servidor. Una carencia reproducida de la ABI se convierte en tarea independiente.

Entregar `target/self-improvement/tasks/T16/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente** (humo loopback guest con T18). Condición de entrada **T14 go** sin cumplir. Evidencia: `target/self-improvement/tasks/T16/resultado.md`, `seguimiento/T16.md`.
