# T24 — Preparar una copia de tarea y exportar su parche

**Hito:** SI-4 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T23](T23-estado-coordinador.md)

## Objetivo y entrega

Prepare produce una copia exacta y diff aplicable sin modificar el repositorio original.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [tools/soso-forja-server/src/main.rs](../../tools/soso-forja-server/src/main.rs)

## Archivos que se pueden cambiar

Crear `tools/soso-improve/src/workspace.rs` y `tests/workspace.rs`; conectar `prepare` al CLI de T23.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Preparar worktree o clon aislado de una base capturada T01; incluir archivos nuevos de esa base de forma verificable, sin tocar el checkout activo.
2. Materializar TASK.md desde TaskSpec; copiar únicamente entradas visibles. Verificadores reservados y estado del coordinador quedan fuera.
3. Canonicalizar rutas, detectar escapes por symlink y rechazar destino existente ajeno. Registrar ruta, base y hash del paquete de tarea.
4. Exportar diff con archivos añadidos, borrados y binarios; distinguir cambios previos de cambios hechos por el intento.
5. Gestionar limpieza solo de directorios registrados por esta ejecución, preservando intentos fallidos para diagnóstico.

## Comprobación

`cargo test -p soso-improve --test workspace`. Repo temporal con archivos staged/unstaged, nuevos/borrados, subruta con espacios, symlink saliente y destino ocupado; reconstruir candidato y comparar contenido.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

El aislamiento de procesos se configura en T25; no presentar el worktree como sandbox.

Entregar `target/self-improvement/tasks/T24/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

