# T47 — Ejecutar procesos con argumentos, canales y límites exactos

**Hito:** SI-4 · **Tipo:** Implementación portable y validación guest · **Estado:** pendiente.

**Dependencias:** [T45](T45-cli-capacidades.md), [T48](T48-reloj-red.md)

## Objetivo y entrega

El adaptador guest cumple Orden/Salida o informa una capacidad ausente; nunca ignora campos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)
- [user/libsoso/src/sys.rs](../../user/libsoso/src/sys.rs)

## Archivos que se pueden cambiar

Adaptador de procesos en user/soso-improve, fixtures ejecutables guest mínimos y contrato de capacidades; ABI/kernel solo mediante fichas derivadas si es necesario.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Reproducir con un hijo de prueba argv vacío/espacios/Unicode, env, cwd y stdin. Auditar spawn_io_ex, getcwd, wait y kill existentes; documentar límites reales.
2. Preservar límites de argv y entorno. Prohibir join(" ") y shell interpolado; si la ABI requiere ampliación, especificar una ficha con estructura, longitudes y errores.
3. Separar stdout/stderr y drenarlos incrementalmente con límites mientras se escribe stdin. Cerrar todos los descriptores y restaurar cwd también ante error; preferir cwd por proceso si existe.
4. Añadir deadline monotónico, cancelación y espera asociada al hijo correcto. Registrar identidad de ejecución que sobreviva a PID reciclado; no atribuir wait de cualquier hijo al intento.
5. Demostrar limpieza de descendientes mediante mecanismo nativo o instancia candidata dedicada. Si no es verificable, bloquear ejecución de candidatos, sin asumir grupos POSIX.

## Comprobación

Hijo fixture con mucho stderr, stdin binario, argumentos con espacios, bloqueo, dos hijos simultáneos y descendiente persistente. Verificar bytes, exit code, timeout, cwd posterior y ausencia de huérfanos dentro de soso. Cada capacidad ausente debe fallar antes del efecto.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [ ] Entrega y comprobaciones terminadas con evidencia.
- [ ] Catálogo, índice y seguimiento sincronizados.
- [ ] Validación nativa registrada por separado, sin inferirla de la compilación.

No portar OpenCode aquí. Dividir los huecos de argv, espera y cancelación en fichas con sondas independientes antes de modificar ABI/kernel.

Resumen durable en seguimiento/T47.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T47/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
