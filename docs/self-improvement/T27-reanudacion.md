# T27 — Reanudar tras caída sin repetir efectos

**Hito:** SI-4 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T23](T23-estado-coordinador.md), [T25](T25-ejecutor.md), [T26](T26-validador.md)

## Objetivo y entrega

Misma base final y mismos efectos que una ejecución sin cortes, o bloqueo explícito sin duplicación.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [tools/soso-forja-server/src/main.rs](../../tools/soso-forja-server/src/main.rs)

## Archivos que se pueden cambiar

Crear `tools/soso-improve/src/recovery.rs` y `tests/recovery.rs`; conectar `resume`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Registrar intención y resultado para lanzamiento, exportación de parche, validación y promoción; dar a cada operación una clave estable.
2. En resume cargar último estado válido y reconciliar con archivos, hashes y procesos propios antes de reejecutar.
3. Si una promoción se realizó antes de escribir resultado, reconocerla por referencia+hash; no crear otro commit ni avanzar dos veces.
4. Si no puede determinarse una acción, dejar estado bloqueado con evidencia y la comprobación necesaria para resolverlo.
5. Limitar intentos acumulados a través de reinicios; reanudar no reinicia el presupuesto.

## Comprobación

`cargo test -p soso-improve --test recovery`. Inyectar corte antes y después de cada escritura durable y operación; PID reciclado; diario truncado; promoción ya aplicada; presupuesto agotado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No convertir operaciones inciertas en pendientes y ejecutarlas ciegamente otra vez.

Entregar `target/self-improvement/tasks/T27/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

