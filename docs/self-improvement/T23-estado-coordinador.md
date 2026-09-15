# T23 — Crear el formato de tareas y estados del coordinador

**Hito:** SI-4 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T01](T01-base.md), [T02](T02-banco.md)

## Objetivo y entrega

Formato estable y transiciones durables; binario no realiza aún builds ni invoca OpenCode.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [Cargo.toml](../../Cargo.toml)
- [tools/soso-forja-server/Cargo.toml](../../tools/soso-forja-server/Cargo.toml)

## Archivos que se pueden cambiar

Crear `tools/soso-improve/{Cargo.toml,src/lib.rs,src/main.rs,src/state.rs,tests/state.rs}`; registrar workspace.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Crear CLI mínima con comandos prepare/run/validate/resume/report aún no implementados devolviendo error explícito donde falte lógica, nunca éxito simulado.
2. Definir TaskSpec, RunManifest, Attempt y State versionados; cada comando de prueba es argv+cwd+timeout, no shell libre.
3. Definir transiciones permitidas y autoridad del validador para accepted. Persistir referencias a artefactos y no grandes logs dentro del estado.
4. Validar límites, IDs y rutas; permitir desconocidos de medición como null con motivo, no cero que parezca medido.
5. Serializar a disco con temporal+flush+rename; usar reloj/identificador inyectables en tests.

## Comprobación

`cargo test -p soso-improve --test state`. Ida/vuelta, versión desconocida, transición inválida, salida truncada, colisión de run id y fallo de escritura sin perder estado anterior.

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

