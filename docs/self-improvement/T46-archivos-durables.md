# T46 — Acreditar persistencia y actualización de referencias en sosofs

**Hito:** SI-4 · **Tipo:** Implementación portable y validación guest · **Estado:** pendiente.

**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

Operaciones durables pequeñas y verificables que pueda usar el estado T23 sin std::fs.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/libsoso/src/sys.rs](../../user/libsoso/src/sys.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Añadir contrato de persistencia al core y adaptadores en tools/soso-improve y user/soso-improve; sonda guest y fixtures de cortes.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Inventariar rename, fsync, close y creación exclusiva de la ABI existente; comprobar errores y persistencia real, sin asumir semántica POSIX por el nombre.
2. Definir escribir temporal, sincronizar y publicar referencia con precondición de hash. Especificar recuperación de temporales y escritor único o exclusión verificable.
3. Implementar el adaptador host y el de sosofs. Si no hay una primitiva necesaria, producir una ficha de ABI/kernel y mantener la capacidad deshabilitada.
4. Inyectar fallos antes y después de cada fase. Al arrancar, seleccionar únicamente una referencia íntegra cuyos objetos y hash existan.
5. Conservar la referencia anterior hasta acreditar la nueva; propagar fallos de escritura, cierre, sincronización y renombrado.

## Comprobación

Pruebas compartidas de truncamiento, espacio agotado y colisión de escritores. En soso reiniciar tras cortes controlados en cada fase y verificar estado anterior o nuevo completo, nunca mezcla. Conservar logs del arranque y hashes; un mock no acredita durabilidad.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [ ] Entrega y comprobaciones terminadas con evidencia.
- [ ] Catálogo, índice y seguimiento sincronizados.
- [ ] Validación nativa registrada por separado, sin inferirla de la compilación.

Una entrega parcial del contrato no cierra la ficha si los adaptadores siguen bloqueados; native_validation permanece pendiente hasta la prueba real de persistencia. Separar cada hueco de kernel en una ficha concreta.

Resumen durable en seguimiento/T46.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T46/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
