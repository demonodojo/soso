# T64 — `sosh` no entiende comillas: una ruta con espacios es inescribible

**Hito:** SI-4 · **Tipo:** Corrección de userspace (shell) · **Estado:** completada (2026-09-24).

**Dependencias:** [T62](T62-argv-en-los-programas.md). **La origina:** T62, paso 4.

## Problema

Desde [T62](T62-argv-en-los-programas.md) el argv viaja entero de punta a
punta: `sosh` pasa las palabras que tokenizó, `libsoso` y el kernel no las
vuelven a juntar, y `main` las recibe separadas. Un programa puede recibir
`/tmp/con espacio.txt` como **un** argumento, y la sonda
`argv/ruta-con-espacios` lo acredita dentro de soso.

Lo que sigue sin poderse es **escribirlo desde la shell**. `tokenize`
(`user/sosh/src/main.rs`) corta en cualquier `char::is_whitespace` y no conoce
`'`, `"` ni `\`:

```
$ cat "/tmp/con espacio.txt"
cat: "/tmp/con: no existe
cat: espacio.txt": no existe
```

Las comillas ni siquiera se quitan: llegan como parte de la palabra. Para el
usuario, la diferencia con el fallo de T62 es ninguna; el sitio donde se pierde
el argumento, sí.

## Por qué no se hizo en T62

T62 cambió **cómo se transporta** un argv que ya existía. Esto es distinto:
hay que decidir una **sintaxis** —qué comillas, si hay escapes, qué pasa con
`\` dentro de comillas dobles, si `'` es literal como en POSIX— y cada opción
compromete a la shell para siempre. Mezclarlo con el transporte habría
escondido una decisión de diseño dentro de una corrección mecánica.

## Decisión tomada: subconjunto POSIX con las dos comillas

| Escrito | Resultado |
|---|---|
| `'…'` | literal hasta la `'` de cierre; dentro **no hay escapes** |
| `"…"` | literal salvo `\"` y `\\` |
| `\X` fuera de comillas | `X` literal (`\ ` es un espacio, `\|` una barra) |
| comilla sin cerrar | **error**, no una palabra a medias |

**Por qué las dos, si en sosh hacen lo mismo.** sosh no expande variables, así
que `'` y `"` son hoy idénticas. Eso podría parecer motivo para implementar
sólo una; es al revés. Precisamente porque no hay diferencia semántica,
aceptar las dos no cuesta nada y **evita la trampa**: quien escriba
`'con espacio.txt'` en una shell que sólo entiende `"` recibe la comilla dentro
del argumento y un `no existe 'con` — el mismo fallo silencioso que
[T62](T62-argv-en-los-programas.md) acaba de quitar, con otro disfraz.

Lo que **no** se hace es fingir que `"` expande. Cuando llegue `$`, `"` tendrá
que expandir y `'` no; esta elección deja el sitio preparado sin prometer nada
hoy.

## Alcance

`user/sosh/src/main.rs` (`tokenize` y los tipos de `Token`). El resto del
camino ya está: `CmdSpec.args` es `Vec<String>` y se pasa como argv.

## Pasos

1. Elegir sintaxis y dejarlo escrito.
2. Implementar en `tokenize`, quitando las comillas de la palabra resultante.
3. Comillas sin cerrar: error claro, no una palabra a medias.
4. Prueba en el shard `sys`: `cat "/tmp/t64/con espacio.txt"` desde sosh, que
   **falle** antes del cambio.
5. Revisar que las redirecciones y el pipe siguen partiéndose igual, y que una
   ruta entrecomillada tras `>` funciona.

## Resultado

`tokenize` devuelve `Result` y entiende comillas y escapes. `Token` gana
`entrecomillada`, que hace falta para dos cosas que si no se pierden en
silencio:

- **Una palabra entrecomillada nunca es un operador.** El reconocimiento de
  descriptor —un dígito suelto pegado a `>`— sólo mira palabras sin comillas,
  así que `"2>"` es un nombre de fichero y no una redirección.
- **`""` es un argumento vacío**, no ningún argumento: la misma distinción que
  T62 tuvo que rescatar.

Dentro de `"`, sólo `\"` y `\\` son escapes; cualquier otra barra se queda
literal, barra incluida, para que una ruta no se coma sus separadores.

## Comprobación

    cargo xtask test sys --only="comillas"     OK con el arreglo, FALLO sin él
    cargo xtask test                           TODO OK (39 pasos)
    cargo xtask check                          TODO OK

El paso nuevo `sosh: comillas en rutas con espacios` **escribe** el fichero por
redirección con la ruta entrecomillada y lo vuelve a **leer**, con las dos
comillas; comprueba además que un fichero inexistente da un error con la ruta
entera y que una comilla sin cerrar se rechaza diciéndolo.

Control negativo, desactivando sólo el reconocimiento de comillas:

    $ cat "/tmp/t64/con espacio.txt"
    cat: "/tmp/t64/con: no existe
    cat: espacio.txt": no existe

Evidencia en `target/self-improvement/tasks/T64/`.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite.** No hay expansión de variables, así que `"` no expande nada; es una
coincidencia de hoy, no una promesa. Y una primera pasada de la suite dio un
timeout de arranque en `[reclaim]`: repetido el shard solo y la suite entera,
39/39. Es la intermitencia por carga ya conocida —el mensaje es «no apareció
sosh —», de antes de que el tokenizador se ejecute—, no una regresión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Es la shell de soso y la prueba se escribe
desde ella, así que no hay otro sitio donde comprobarlo. Validación nativa:
**verificada** (2026-09-24).
