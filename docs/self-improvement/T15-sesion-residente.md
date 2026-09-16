# T15 — Extraer la sesión residente manteniendo ask

**Hito:** SI-2 · **Tipo:** Refactor guest · **Estado:** pendiente.

**Dependencias:** [T08](T08-resultado-generacion.md), [T09](T09-cancelacion-runtime.md)

## Objetivo y entrega

ask carga una vez entre dos preguntas y reconexión; dos pools sucesivos terminan. No hay cambios de tokens por el refactor.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2, C4**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [user/soso-llm/src/main.rs](../../user/soso-llm/src/main.rs)
- [user/soso-llm/src/ask.rs](../../user/soso-llm/src/ask.rs)
- [user/soso-llm/src/pool.rs](../../user/soso-llm/src/pool.rs)
- [user/soso-llm/src/staging.rs](../../user/soso-llm/src/staging.rs)

## Archivos que se pueden cambiar

Crear `user/soso-llm/src/session.rs`; editar main.rs y ask.rs solo para extraer/reutilizar la sesión.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Localizar Sesion, preparación, carga, hooks y liberación de pool. Extraer un propietario de sesión que no dependa del formato del socket de salida.
2. Proporcionar selección estricta para API y conservar selección histórica de ask. Mantener una carga por proceso y reset de secuencia por petición.
3. Separar salida de diagnóstico de callbacks de tokens. Preparación HTTP nunca recibe fd_out del protocolo textual de ask.
4. Restaurar hooks y apagar workers en todos los retornos. Conservar staging async apagado según el código existente.
5. Conectar ask al propietario extraído sin añadir todavía HTTP ni cambiar el texto/protocolo de ask.

## Comprobación

Con cwd `user/`: `cargo build --release -p soso-llm`. Desde raíz: `cargo xtask test -- --guest llm-dense --only ask` y verificar que se ejecuta el caso de modelo residente; consultar el filtro actual si cambió.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si existe una regresión previa de ask, conservar evidencia y separar su arreglo antes de declarar el refactor equivalente.

Entregar `target/self-improvement/tasks/T15/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
