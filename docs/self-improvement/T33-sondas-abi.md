# T33 — Crear sondas pequeñas para las capacidades requeridas

**Hito:** SI-6 · **Tipo:** Implementación de pruebas guest · **Estado:** completada (2026-09-24).

**Dependencias:** [T32](T32-opencode-inventario.md), [T49](T49-pruebas-guest.md)

## Objetivo y entrega

Todas las capacidades identificadas tienen sonda y resultado; cada fallo tiene reproducción independiente.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

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

## Resultado

Siete sondas en `user/soso-agent-probe/`, un módulo por capacidad, ejecutadas en
**siete pases** —uno por vez, como la ficha exige— y con el resumen en
[`native/probes.json`](native/probes.json), que lleva `esperado` y `observado`
por caso **también en los que pasan**, para que el criterio se pueda revisar
después.

### Lo que sale bien, y no era obvio

Páginas ejecutables **con recompilación** (el JIT es viable), señales
suficientes para el timeout de una herramienta, stdout y stderr separados,
hilos con `join` que espera de verdad, relojes que no mienten, tuberías que
aguantan 64 KiB y avisan del EOF cuando el escritor muere, y TCP que reconecta
al mismo destino.

### Lo que sale mal, con reproducción

- **El modelo de ficheros es por descriptor, no por inodo** (pase 3): dos
  descriptores no comparten el fichero y una escritura parcial **trunca la
  cola**. SQLite no puede funcionar así, y el motivo no es que falte `pread`
  —`seek`+`read` lo sustituye— sino el modelo entero.
- **No hay protecciones de página más allá de la escritura** (pases 2 y 6). No
  existe `PROT_EXEC` porque todo es ejecutable, y no existe `PROT_NONE`, así
  que no puede haber guarda de pila. Es **una** decisión de diseño con los dos
  signos: hace viable el JIT e imposible detectar un desbordamiento de pila.

### Dos defectos abiertos aparte

**[T65](T65-o-excl-no-excluye.md)** — `O_EXCL` no excluye mientras el primer
descriptor sigue abierto, y de él depende el contrato durable de
[T46](T46-archivos-durables.md). **[T66](T66-guarda-de-pila-fingida.md)** — la
guarda que `thread::spawn` dice instalar y no instala.

### Lo que queda listo para T34

Cinco huecos con archivos responsables y reproducción, en el campo `para_t34`
de cada sonda.

## Comprobación

Build de `soso-agent-probe` con cwd `user/`; ejecución de cada subcomando dentro de QEMU con marker y exit code. Reiniciar entre pruebas de persistencia. No basta ejecutar pruebas host de wrappers.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Cómo se cumplió «una receta por capacidad, una ejecución pequeña cada vez».**
Siete pases, cada uno con su sonda, su ejecución en QEMU y su entrada en el
seguimiento. El conjunto **no** se marcó tras la primera.

**Límite.** Sólo la sonda de archivos tiene variante con reinicio real
(`cargo xtask test-probe`). Las demás miden dentro de un arranque, que es lo
que sus capacidades necesitan; si alguna pasara a depender de la persistencia,
habría que añadirle su par de fases.

La ficha es una receta por capacidad, una ejecución pequeña cada vez. No marcar el conjunto completo tras implementar solo la primera sonda.

Entregar `target/self-improvement/tasks/T33/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **verificada** (2026-09-24). Las siete sondas corren dentro
de soso, y la de archivos además con un `halt` y un arranque nuevo en medio.
Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
