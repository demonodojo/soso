# T24 — Preparar una copia de tarea y exportar su parche

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** completada (2026-09-24).

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

## Resultado

`crates/soso-improve-core/src/workspace.rs`: `preparar` reconstruye la base de
[T01](T01-base.md) en otro sitio y siembra el enunciado; `exportar` devuelve un
paquete de [T50](T50-cambios-contenido.md) aplicable al árbol original. Órdenes
`tarea copiar` y `tarea exportar` en el CLI compartido y en el host; sonda
`workspace/copia-y-parche` en el guest.

Las decisiones que esta ficha fija:

- **Lo que siembra el coordinador no es un cambio del intento.** El `TASK.md` lo
  escribimos nosotros; exportado contra la base aparecería como un alta del
  candidato y acabaría aplicándose al repositorio de verdad. La copia registra
  qué plantó y se descuenta — también si el candidato lo modifica.
- **Un enlace simbólico se informa, no se descarta.** El recorrido ya lo dejaba
  fuera del inventario; lo que faltaba era que **apareciera**. Un enlace a
  `/etc/passwd` que desaparece del parche sin dejar rastro es cómo su contenido
  acaba colándose sin que nadie lo vea.
- **Un destino ocupado no se pisa ni se borra.** Sin marca y con contenido, se
  rechaza; con la marca de otra ejecución, se rechaza nombrándola; con la de la
  misma, se reutiliza, porque eso es reanudar.
- **`limpiar` comprueba la marca antes de tocar nada**, y `conservar` deja en
  pie las copias de los intentos no aceptados: son lo único que queda para
  saber qué pasó.

Un cambio fuera del módulo, mínimo: `captura::NO_REGULAR`. `workspace`
distinguía lo sospechoso de lo ignorado comparando contra un **literal**, y con
dos literales iguales reescribir uno haría desaparecer los enlaces del informe
sin que fallara nada — el fallo exacto que el módulo dice evitar.

## Comprobaciones ejecutadas

    cargo test -p soso-improve-core --features std --test workspace   14/14
    cargo build -p soso-improve-core --no-default-features            ok (C1)
    cargo xtask test                                                  TODO OK (38 pasos)
    cargo xtask check                                                 TODO OK

Ciclo real del host sobre `crates/soso-improve-core` y salida del guest en
`target/self-improvement/tasks/T24/`.

### Las pruebas se auditaron con mutación

Las 14 pasaron a la primera, así que se quitó cada guarda por turnos. Dos eran
de verdad; el filtro de rechazados **no**: el caso usaba `target/debug/basura.o`
y `target` se poda como **directorio**, así que su contenido nunca se recorría y
el filtro no discriminaba nada. Cambiado a `models/pesos.gguf`, que se excluye
por prefijo siendo archivo.

## Límite

El diff de Git «opcional» del paso 4 no se hace: `Paquete.git` queda a `None`.
No hay Git en soso, y añadirlo sólo en el host crearía una asimetría que después
habría que quitar.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

El aislamiento de procesos se configura en T25; no presentar la copia de archivos como sandbox.

Entregar `target/self-improvement/tasks/T24/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **verificada** (2026-09-24). La sonda
`workspace/copia-y-parche` corre el ciclo entero dentro de soso —capturar,
copiar, rechazar un destino ajeno, exportar y **aplicar el parche al árbol
original**— sin Git en ningún paso, que es el motivo de que todo esto exista.
Condiciones previas: [T49](T49-pruebas-guest.md), [T50](T50-cambios-contenido.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
