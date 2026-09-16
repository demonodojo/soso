# T50 — Aplicar y exportar cambios sin Git

**Hito:** SI-4 · **Tipo:** Implementación portable y validación guest · **Estado:** pendiente.

**Dependencias:** [T01](T01-base.md), [T46](T46-archivos-durables.md)

## Objetivo y entrega

Paquete versionado de cambios que reconstruye un candidato exacto desde una captura T01 sin git apply.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [crates/soso-improve-core/src/captura.rs](../../crates/soso-improve-core/src/captura.rs)
- [crates/soso-improve-core/src/repo.rs](../../crates/soso-improve-core/src/repo.rs)
- [tools/soso-improve/src/verificar.rs](../../tools/soso-improve/src/verificar.rs)

## Archivos que se pueden cambiar

Módulo delta del core, adaptación del callback de referencias y ambos frontends; esquema y fixtures pequeños.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Definir base esperada, inventario destino y operaciones con ruta/hash antes/hash después; objetos para contenido binario, altas y borrados. Reutilizar almacenamiento por hash de T01.
2. Validar paquete entero y rutas antes de escribir; rechazar duplicados, traversal, enlaces salientes y precondiciones distintas. Construir árbol nuevo y verificar su hash final.
3. Exportar diferencias por inventarios. El commit o diff Git puede adjuntarse como metadato, nunca como requisito para aplicar el cambio.
4. Convertir referencias R del banco mediante herramienta Rust de preparación; conservar fuentes y hashes, versionar el formato y renovar huella del banco con comparación explícita de resultados.
5. Conectar AplicarParche a este formato en host y guest; no cambiar silenciosamente significado ni límites de los casos.

## Comprobación

Roundtrip de texto, binarios, archivos vacíos, borrados y rutas con espacios. Base incorrecta, objeto ausente y fallo de escritura dejan la base original intacta. Aplicar en soso sin Git instalado y comparar hashes con el host.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [ ] Entrega y comprobaciones terminadas con evidencia.
- [ ] Catálogo, índice y seguimiento sincronizados.
- [ ] Validación nativa registrada por separado, sin inferirla de la compilación.

No implementar merge o VCS completo. Un parche Git importado es una entrada de preparación; la campaña recibe el paquete portable ya validado.

Resumen durable en seguimiento/T50.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T50/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
