# T51 — Acreditar todo el circuito de automejora dentro de soso

**Hito:** SI-7 · **Tipo:** Integración y evaluación nativa · **Estado:** pendiente.

**Dependencias:** [T29](T29-campana.md), [T35](T35-opencode-nativo.md), [T37](T37-mejora-nativa-forja.md), [T41](T41-cargo-offline.md), [T42](T42-c-link-imagen.md), [T43](T43-validacion-actualizacion-nativa.md), [T49](T49-pruebas-guest.md), [T50](T50-cambios-contenido.md)

## Objetivo y entrega

Matriz de ejecución real de cada componente y campaña de aceptación sin dependencias funcionales Linux; puerta obligatoria de T44.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [docs/self-improvement/NATIVO.md](../../docs/self-improvement/NATIVO.md)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Informe native/aceptacion.md, manifiesto de ejecución y configuración de campaña; correcciones mediante sus fichas de implementación.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Fijar perfil, semilla, inventarios de herramientas y fixtures. Exigir cierre de todas las fichas N/C necesarias, instalación offline de OpenCode y reconstrucción nativa de herramientas requeridas.
2. Desconectar Forja, modelos externos y ejecutores Linux. Ejecutar desde soso captura, reconstrucción, banco completo P/Q/R, evaluación, prepare/run/validate/resume/report y consulta de conocimiento.
3. Usar validador independiente en soso y candidato separado según T26/T43. Registrar envío de paquetes, identidad de pruebas y control nativo de arranque/recuperación.
4. Repetir campaña T29 con mismas reglas y denominadores; incluir timeout, caída de red, escritura interrumpida, test fallido y promoción concurrente. Un caso no ejecutable bloquea el cierre.
5. Comparar equivalencia del banco y cobertura de C6, publicar plataformas de cada proceso y actualizar native_validation solo para tareas con evidencia enlazada. Entregar a T44 la configuración reproducible.

## Comprobación

Campaña nativa de diez tareas con ≥5 mejoras aceptadas, recuperación y banco completo sin omisiones. Forzar una dependencia externa indisponible y demostrar fallo explícito si aún se usa; no fallback. T44 exige después tres mejoras consecutivas sobre esta configuración.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [ ] Entrega y comprobaciones terminadas con evidencia.
- [ ] Catálogo, índice y seguimiento sincronizados.
- [ ] Validación nativa registrada por separado, sin inferirla de la compilación.

No arreglar todo el port en esta ficha de integración. Cualquier dependencia funcional externa residual bloquea T51 y T44 con una reproducción y ficha concreta.

Resumen durable en seguimiento/T51.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T51/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
