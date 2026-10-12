# T33 — Crear sondas pequeñas para las capacidades requeridas

**Hito:** SI-6 · **Tipo:** Implementación de pruebas guest · **Estado:** pendiente.  
**Dependencias:** [T32](T32-opencode-inventario.md)

## Objetivo y entrega

Todas las capacidades identificadas tienen sonda y resultado; cada fallo tiene reproducción independiente.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [crates/soso-std/src/process.rs](../../crates/soso-std/src/process.rs)
- [crates/soso-std/src/fs.rs](../../crates/soso-std/src/fs.rs)
- [crates/soso-std/src/net.rs](../../crates/soso-std/src/net.rs)
- [user/soso-std-test/src/main.rs](../../user/soso-std-test/src/main.rs)
- [user/libsoso/src/thread.rs](../../user/libsoso/src/thread.rs)

## Archivos que se pueden cambiar

Crear `user/soso-agent-probe/` y registrar workspace; módulo separado por capacidad. Tabla de resultados en `docs/self-improvement/native/probes.json`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar primero una sonda de archivos persistentes: crear, escribir, cerrar, reabrir y comparar Unicode/binario. Reportar fase y código exacto.
2. En pases separados de esta ficha, añadir una sonda por requisito T32: argv/env/cwd, stdout/stderr y exit status, pipes con EOF, thread/join/TLS, temporizadores y TCP con cierre/reconexión.
3. Cada pase toca un módulo de sonda y su registro; ejecutar solo ese caso más humo del binario. No mezclar arreglos del kernel con la introducción del test.
4. Registrar expected/observed y evidencia; detectar stubs que devuelven éxito sin efecto. Bibliotecas requeridas como SQLite necesitan su propia sonda según T32.
5. Una capacidad fallida produce entrada de backlog T34 con reproducción mínima y archivos responsables.

## Comprobación

Build de `soso-agent-probe` con cwd `user/`; ejecución de cada subcomando dentro de QEMU con marker y exit code. Reiniciar entre pruebas de persistencia. No basta ejecutar pruebas host de wrappers.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

La ficha es una receta por capacidad, una ejecución pequeña cada vez. No marcar el conjunto completo tras implementar solo la primera sonda.

Entregar `target/self-improvement/tasks/T33/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

