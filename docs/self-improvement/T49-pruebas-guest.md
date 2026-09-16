# T49 — Ejecutar casos compartidos desde un runner nativo

**Hito:** SI-0 / SI-4 · **Tipo:** Implementación portable y validación guest · **Estado:** pendiente.

**Dependencias:** [T45](T45-cli-capacidades.md), [T46](T46-archivos-durables.md), [T47](T47-procesos-nativos.md), [T48](T48-reloj-red.md)

## Objetivo y entrega

Un runner guest consume fixtures versionados y devuelve aserciones, fallos y capacidades pendientes sin necesitar libtest.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)
- [tests/self-improvement/cases/banco.json](../../tests/self-improvement/cases/banco.json)

## Archivos que se pueden cambiar

Funciones de prueba compartidas no_std, despacho de pruebas en user/soso-improve y wrappers host; manifiesto de fixtures.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Extraer aserciones reutilizables de captura/banco/protocolo y mecanismos T45–T48 a funciones sin libtest; conservar tests host como wrappers.
2. Definir subcomando propuesto soso-improve pruebas con entrada de manifiesto y salida JSON: id, target, estado, exit code, hashes, capacidad pendiente y duración.
3. Instalar el ejecutable guest y fixtures desde la semilla de desarrollo. Comparar captura/reconstrucción por contenido y resultados host/guest.
4. Registrar programa/repo pendientes hasta T40/T41: no contarlos como aprobados ni excluirlos del total. Mantener huella del banco o versionar explícitamente cualquier adaptación.
5. Probar casos negativos y salida truncada. Permitir que T19 y T33 registren nuevas suites sin duplicar runner.

## Comprobación

Arrancar soso y ejecutar captura/reconstrucción, banco y protocolo, más pruebas de adaptadores. Acreditar exit no cero al fallar una aserción. No es obligatorio disponer de compilador en este punto; distinguir fixtures precompilados de candidatos compilados. T51 completará P/R.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [ ] Entrega y comprobaciones terminadas con evidencia.
- [ ] Catálogo, índice y seguimiento sincronizados.
- [ ] Validación nativa registrada por separado, sin inferirla de la compilación.

El cierre de esta ficha acredita el runner y las suites disponibles; no cierra la evaluación completa ni T01/T02 nativos si quedan capacidades sin ejecutar.

Resumen durable en seguimiento/T49.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T49/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
