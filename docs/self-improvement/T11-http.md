# T11 — Leer HTTP fragmentado con límites explícitos

**Hito:** SI-2 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.  
**Dependencias:** [T10](T10-api-json.md)

## Objetivo y entrega

El parser consume bytes exactos y tiene memoria acotada; las entradas incompletas nunca se aceptan como completas.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [tools/soso-forja-server/src/main.rs](../../tools/soso-forja-server/src/main.rs)
- [user/soso-llm/src/net.rs](../../user/soso-llm/src/net.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-api/src/http.rs` y `tests/http.rs`; exponer en `lib.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar un parser incremental puro: NeedMore, Complete(request) o Error. Alimentarlo con bytes sin sockets ni reloj dentro.
2. Reconocer fin de cabeceras y Content-Length case-insensitive; comprobar sumas, duplicados ambiguos, cuerpos incompletos y límites antes de reservar.
3. Aceptar una petición por conexión. Rechazar Transfer-Encoding y ambigüedades de framing; no implementar keep-alive ni pipelining.
4. Dejar que el llamante comunique EOF y timeout; distinguir cuerpo incompleto de petición válida de longitud cero.
5. Serializar cabeceras de respuesta y Content-Length exacto. Para SSE usar cierre de conexión como framing, sin Content-Length falso.

## Comprobación

`cargo test -p soso-llm-api --features std --test http`. Cada punto de fragmentación, cabecera dividida, cuerpo binario no JSON todavía, tamaño exacto/+1, overflow, longitud duplicada contradictoria, EOF y petición extra.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

El lector Forja es contexto, no una implementación que copiar con sus defaults; no modificar Forja en esta ficha.

Entregar `target/self-improvement/tasks/T11/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

