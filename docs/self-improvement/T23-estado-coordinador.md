# T23 — Crear el formato de tareas y estados del coordinador

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** completada (2026-09-24).

**Dependencias:** [T01](T01-base.md), [T02](T02-banco.md), [T46](T46-archivos-durables.md), [T45](T45-cli-capacidades.md)

## Objetivo y entrega

Formato estable y transiciones durables; binario no realiza aún builds ni invoca OpenCode.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Extender las crates existentes: estado en `crates/soso-improve-core/src/state.rs`, wrappers en `tools/soso-improve` y `user/soso-improve`, pruebas compartidas. No recrear crates ni duplicar política entre frontends.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Extender la CLI compartida T45 con comandos prepare/run/validate/resume/report aún no implementados devolviendo error explícito donde falte lógica, nunca éxito simulado.
2. Definir TaskSpec, RunManifest, Attempt y State versionados; cada comando de prueba es argv+cwd+timeout, no shell libre.
3. Definir transiciones permitidas y autoridad del validador para accepted. Persistir referencias a artefactos y no grandes logs dentro del estado.
4. Validar límites, IDs y rutas; permitir desconocidos de medición como null con motivo, no cero que parezca medido.
5. Persistir mediante el contrato durable T46, con reloj/identificador inyectables. Probar también el adaptador sosofs; no llamar std::fs desde core.

## Comprobación

`cargo test -p soso-improve-core --test state`. Ida/vuelta, versión desconocida, transición inválida, salida truncada, colisión de run id y fallo de escritura sin perder estado anterior.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Resultado

El formato vive en `crates/soso-improve-core/src/state.rs`: `TaskSpec`,
`RunManifest`, `Attempt`, `Estado`, `Autoridad`, `Medida`, `Comprobacion` y
`Limites`, con las transiciones permitidas y la persistencia sobre el contrato
durable de [T46](T46-archivos-durables.md) (`guardar`, `cargar`), reloj e ids
inyectables. Las órdenes (`preparar`, `reanudar`, `informe`) están también en el
crate portable: si el criterio se duplicara entre frontends, una campaña diría
cosas distintas según dónde corriera.

Las decisiones que esta ficha fija, y que después no se pueden cambiar sin
romper estados ya escritos:

- **Sólo `Autoridad::Validador` escribe `Aceptada`.** Un coordinador que aprueba
  su propio trabajo no valida nada. La autoridad viaja en la transición.
- **Lo que no se midió se dice, no se pone a cero.** `Medida::Desconocida` lleva
  motivo, y no hay constructor que acepte un `u64` y ponga cero por defecto:
  esa comodidad es cómo aparece un cero inventado en un informe.
- **En el estado van referencias, no logs**, con tope de longitud y de número.
- **Las comprobaciones son `argv` + `cwd` + plazo** (C5), y un plazo de cero se
  rechaza.
- Los ids se validan porque acaban siendo nombres de fichero: `../` no pasa.
- `reanudar` **informa y no continúa nada**; continuar sin repetir efectos es
  [T27](T27-reanudacion.md), y la salida de la orden lo dice.
- `ejecutar` y `validar` existen en el CLI compartido y se rechazan nombrando
  [T25](T25-ejecutor.md) y [T26](T26-validador.md). No se simula éxito:

      $ soso-improve tarea ejecutar --estado … --run r-demo
      error: uso: capacidad ausente en host: «tarea ejecutar» llega en T25

## Comprobaciones ejecutadas

    cargo test -p soso-improve-core --features std --test state   14/14
    cargo build -p soso-improve-core --no-default-features        ok (C1)
    cargo xtask test sys --only="estado de tareas"                TODO OK

El ciclo real del host está en
`target/self-improvement/tasks/T23/host-ciclo.txt`.

## Límite: la prueba destapó un defecto de T46

Una escritura cortada **no** pierde el estado anterior —eso funciona—, pero deja
la referencia **encallada**: `publicar` numera la generación siguiente desde la
vigente íntegra y choca con el fichero roto, que sigue en el disco. Y falla así
para siempre.

Era defecto de T46 y quedaba fuera del alcance de esta ficha, así que se abrió
**[T63](T63-generacion-rota-encalla.md)** con la reproducción — y se **cerró el
mismo día**. La prueba de aquí, que fijaba a propósito la conducta de entonces,
está invertida: ahora exige que la escritura tras el corte pase y que el estado
avance. Con eso **[T27](T27-reanudacion.md)** deja de ser imposible por
construcción.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

No implementar todo el coordinador en esta ficha; T24–T28 completan cada capacidad.

Entregar `target/self-improvement/tasks/T23/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **verificada** (2026-09-24). El estado se escribe y se
recupera **desde sosofs**, no desde una struct en memoria: el paso de suite
`soso-improve: estado de tareas` prepara, avanza, cierra un intento con una
medida desconocida, vuelve a leerlo todo del disco y comprueba que el
coordinador sigue sin poder aceptar y que la medida ausente sigue ausente.
Condiciones previas: [T46](T46-archivos-durables.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
