# T45 — Unificar órdenes, capacidades y códigos de salida

**Hito:** SI-0 · **Tipo:** Implementación portable y validación guest · **Estado:** hecha (2026-09-23).

**Dependencias:** [T01](T01-base.md), [T02](T02-banco.md)

## Objetivo y entrega

Una orden y un resultado compartidos; ambos frontends rechazan funciones ausentes antes de ejecutar efectos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear un módulo de órdenes/resultados en core y adaptar los dos main existentes; fixtures CLI pequeños.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Inventariar sintaxis host/guest y códigos actuales. Definir una orden serializable y aliases compatibles; documentar qué nombres quedan obsoletos.
2. Compartir despacho y clasificación de resultados: 0 éxito, 1 error/operación no disponible, 2 verificación fallida, 3 captura inestable, 4 suite fallida. Reutilizar las convenciones host existentes.
3. Propagar errores de reconstrucción, banco y protocolo hasta el exit code guest; imprimir FAIL nunca puede terminar con éxito.
4. Publicar capacidades realmente implementadas. Rechazar programa/repo sin compilador, o campos de Orden no soportados, antes del primer efecto.
5. Para órdenes complejas admitir entrada JSON por archivo con límites; no suponer que split_whitespace preserva argv. T47 resolverá el transporte de argumentos a procesos.

## Comprobación

Probar en ambos frontends: captura válida, objeto corrupto, banco inválido, protocolo incorrecto y capacidad ausente. Comparar resultado estructurado y exit code. Para esta tarea bastan fixtures preinstalados y ejecución guest; no hace falta Cargo nativo.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [x] Entrega y comprobaciones terminadas con evidencia.
- [x] Catálogo, índice y seguimiento sincronizados.
- [x] Validación nativa registrada por separado, sin inferirla de la compilación.

Entregado en `crates/soso-improve-core/src/cli.rs` (`Codigo`, `Capacidad`,
`Capacidades`, `Orden`, `Informe`, `argv_de_texto`), con los dos frontends
adaptados y fixtures en `rootfs/var/self-improvement/fixtures-cli/`. El paso
«soso-improve: órdenes y códigos de salida» de `cargo xtask test` lo ejecuta
dentro de soso. Resumen en [seguimiento/T45.md](seguimiento/T45.md); evidencia
en `target/self-improvement/tasks/T45/resultado.md`.

**Límites.** El interior de cada subcomando del host conserva su parser
`Opciones`; lo compartido es la orden canónica, la capacidad y el código. El
transporte de `Orden` a un proceso sigue siendo [T47](T47-procesos-nativos.md).
`native_validation` queda en **partial**: se ejecuta en guest, pero el guion lo
lanza el arnés del host, y repetirlo desde el runner nativo es
[T49](T49-pruebas-guest.md).

No implementar aquí nuevos mecanismos de procesos o filesystem. Si falta una syscall, registrar una ficha derivada.

Resumen durable en seguimiento/T45.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T45/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
