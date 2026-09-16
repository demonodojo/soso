# T23 — Crear el formato de tareas y estados del coordinador

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T01](T01-base.md), [T02](T02-banco.md), [T46](T46-archivos-durables.md), [T45](T45-cli-capacidades.md)

## Objetivo y entrega

Formato estable y transiciones durables; binario no realiza aún builds ni invoca OpenCode.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Extender las crates existentes: estado en `crates/soso-improve-core/src/state.rs`, wrappers en `tools/soso-improve` y `user/soso-improve`, pruebas compartidas. No recrear crates ni duplicar política entre frontends.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Extender la CLI compartida T45 con comandos prepare/run/validate/resume/report aún no implementados devolviendo error explícito donde falte lógica, nunca éxito simulado.
2. Definir TaskSpec, RunManifest, Attempt y State versionados; cada comando de prueba es argv+cwd+timeout, no shell libre.
3. Definir transiciones permitidas y autoridad del validador para accepted. Persistir referencias a artefactos y no grandes logs dentro del estado.
4. Validar límites, IDs y rutas; permitir desconocidos de medición como null con motivo, no cero que parezca medido.
5. Persistir mediante el contrato durable T46, con reloj/identificador inyectables. Probar también el adaptador sosofs; no llamar std::fs desde core.

## Comprobación

`cargo test -p soso-improve-core --test state`. Ida/vuelta, versión desconocida, transición inválida, salida truncada, colisión de run id y fallo de escritura sin perder estado anterior.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No implementar todo el coordinador en esta ficha; T24–T28 completan cada capacidad.

Entregar `target/self-improvement/tasks/T23/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T46](T46-archivos-durables.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
