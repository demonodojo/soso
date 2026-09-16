# T24 — Preparar una copia de tarea y exportar su parche

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T23](T23-estado-coordinador.md), [T50](T50-cambios-contenido.md)

## Objetivo y entrega

Prepare produce una copia exacta y paquete de cambios aplicable sin modificar el repositorio original.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-improve-core/src/workspace.rs` y pruebas compartidas; conectar `prepare` en ambos frontends. Usar captura T01 y delta T50.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Reconstruir un árbol desde la captura por contenido T01, sin Git; verificar inventario y archivos nuevos, sin tocar el árbol activo.
2. Materializar TASK.md desde TaskSpec; copiar únicamente entradas visibles. Verificadores reservados y estado del coordinador quedan fuera.
3. Canonicalizar rutas, detectar escapes por symlink y rechazar destino existente ajeno. Registrar ruta, base y hash del paquete de tarea.
4. Exportar paquete T50 con archivos añadidos, borrados y binarios; distinguir cambios previos de cambios del intento. Un diff Git para revisión es opcional.
5. Gestionar limpieza solo de directorios registrados por esta ejecución, preservando intentos fallidos para diagnóstico.

## Comprobación

`cargo test -p soso-improve-core --test workspace`. Árbol temporal sin .git, con archivos nuevos/borrados, subruta con espacios, symlink saliente y destino ocupado; reconstruir candidato y comparar contenido.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

El aislamiento de procesos se configura en T25; no presentar la copia de archivos como sandbox.

Entregar `target/self-improvement/tasks/T24/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T49](T49-pruebas-guest.md), [T50](T50-cambios-contenido.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
