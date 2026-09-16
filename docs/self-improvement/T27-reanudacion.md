# T27 — Reanudar tras caída sin repetir efectos

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T23](T23-estado-coordinador.md), [T25](T25-ejecutor.md), [T26](T26-validador.md)

## Objetivo y entrega

Misma base final y mismos efectos que una ejecución sin cortes, o bloqueo explícito sin duplicación.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-improve-core/src/recovery.rs` y pruebas compartidas; conectar `resume` en ambos frontends con persistencia T46 e identidad de procesos T47.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Registrar intención y resultado para lanzamiento, exportación de parche, validación y promoción; dar a cada operación una clave estable.
2. En resume cargar último estado válido y reconciliar con archivos, hashes y procesos propios antes de reejecutar.
3. Si una promoción se realizó antes de escribir resultado, reconocerla por referencia+hash; no volver a publicar el paquete ni avanzar dos veces.
4. Si no puede determinarse una acción, dejar estado bloqueado con evidencia y la comprobación necesaria para resolverlo.
5. Limitar intentos acumulados a través de reinicios; reanudar no reinicia el presupuesto.

## Comprobación

`cargo test -p soso-improve-core --test recovery`. Inyectar corte antes y después de cada escritura durable y operación; PID reciclado; diario truncado; promoción ya aplicada; presupuesto agotado.

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

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T46](T46-archivos-durables.md), [T47](T47-procesos-nativos.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
