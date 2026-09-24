# T62 — Los programas reciben los argumentos juntados, no su argv

**Hito:** SI-4 · **Tipo:** Corrección de userspace y kernel · **Estado:** completada (2026-09-24).

**Dependencias:** ninguna. **La origina:** [T47](T47-procesos-nativos.md).

## Problema reproducido

El argv **llega bien** hasta el proceso hijo: el adaptador pasa la tabla, la
syscall la valida (`kernel/src/task/syscall.rs`, `argv_ptr`/`argv_count`) y
`libsoso` la guarda entera en su global `ARGV`. Lo que se pierde es el último
paso: `entry!` le entrega a `main` una **sola cadena** con los argumentos
juntados por espacios, y cada programa la vuelve a partir.

```rust
// user/libsoso/src/lib.rs
pub fn args_for_main(blob: &[u8]) -> String {
    if let Some(argv) = decode_argv(blob) {
        let args = if argv.len() > 1 { argv[1..].join(" ") } else { String::new() };
        *ARGV.lock() = Some(argv);   // ← el argv real se guarda…
        return args;                  // ← …y se devuelve el juntado
    }
    …
}
```

Consecuencia observable, con un archivo cuyo nombre lleva un espacio:

```
$ soso-improve procesos          # con el caso escrito contra `cat`
cat: /tmp/t47: no existe
cat: con: no existe
cat: espacio.txt: no existe
```

El adaptador había pasado **una** ruta; `cat` recibió `"/tmp/t47 con
espacio.txt"` en una cadena y la partió en tres.

## Por qué importa

Cualquier ruta con espacios es inmanejable desde un proceso, y eso incluye lo
que el coordinador de automejora tendrá que ejecutar. Además, la información
para hacerlo bien **ya está ahí**: `libsoso::argv()` devuelve el argv real.

## Contexto mínimo

- `user/libsoso/src/lib.rs`: `ARGV`, `argv()`, `args_for_main`, `entry!`.
- `user/coreutils/src/bin/*.rs`: los que parten la cadena (`cat`, `ls`, `head`,
  `tail`, `grep`, `wc`, `cp`, `mv`, `rm`, `stat`, `diff`, `find`).

## Decisión tomada: camino 1

**Cambiar la firma de `main` a `&[String]`.** La cadena juntada es lossy por
construcción, así que el parámetro era una respuesta cómoda y equivocada: para
hacerlo bien había que **ignorarlo** y llamar a `libsoso::argv()`. Un parámetro
que hay que ignorar para ser correcto no es una comodidad, es una trampa.

Lo que decidió fue el coste en el tiempo, no el coste de una vez: el del camino
1 lo encuentra el compilador —39 binarios, uno a uno— y se paga hoy; el del
incremental es indefinido, porque nada falla al escribir el programa siguiente
y no se sabrá hasta que alguien pase una ruta con un espacio.

Los programas que de verdad quieren una cadena (`echo`, el texto de `ask`) la
construyen con `join(" ")`: lo mismo, pero explícito y sobre el argv de verdad.

## La decisión, tal como estaba planteada

Hay dos caminos y afectan a todos los programas:

1. **Cambiar la firma** de `main` a `&[String]`. Es lo correcto a largo plazo y
   toca todos los binarios de golpe.
2. **Dejar la firma** y que cada programa use `libsoso::argv()` cuando le
   importe. Menos invasivo, pero deja la trampa puesta para el siguiente.

La segunda se puede hacer incrementalmente; la primera conviene hacerla de una
vez. Elegir esto es parte de la ficha, no un detalle de implementación.

## Pasos

1. Elegir camino y dejarlo escrito antes de tocar código.
2. Migrar primero los que el coordinador usa de verdad (`cat`, `ls`, `head`,
   `grep`, `wc`).
3. Prueba en guest con un archivo cuyo nombre lleve espacios, que **falle**
   antes del cambio.
4. Revisar `sosh`: su tokenizador es otra historia —parte por espacios sin
   comillas—, y merece decidirse en la misma pasada o en ficha aparte.

## Resultado: no era un sitio, eran cinco

La ficha decía que el argv «llega bien hasta el proceso hijo» y que sólo fallaba
`entry!`. Eso es cierto **sólo** para `spawn_io_ex`, el camino que abrió T47.
Por los demás el argv nunca existió: metían la **línea entera** como un único
argumento, y el juntado de `entry!` lo compensaba al otro lado.

| Sitio | Qué recibe | Qué hace ahora |
|---|---|---|
| `entry!` | argv | lo entrega tal cual |
| `sosh` | palabras ya tokenizadas | las pasa como argv en vez de rejuntarlas |
| `libsoso::sys::spawn_io` | una línea | la parte |
| kernel `spawn_console` (`SYS_SPAWN`) | una línea | la parte |
| kernel `read_spawn_args` (fallback) | una línea | la parte |

**El compilador no podía encontrar los cuatro últimos**: una línea empaquetada
y un argumento de verdad son los dos un `String`. Salieron en QEMU, con
síntomas que no se parecían a la causa — `errno -61` en un puerto, un fallo en
los registros YMM, y un HTTP 500 de Forja.

## Comprobación

    cargo xtask test                       TODO OK (38 pasos)
    cargo xtask check                      TODO OK

Paso nuevo en el shard `sys`: la sonda `argv/ruta-con-espacios`. **Falla sin el
arreglo**, que es lo que la ficha exigía: revirtiendo sólo `cat` reproduce el
síntoma literal del enunciado,

    argv: cat devolvió "cat: /tmp/t62/con: no existe\ncat: espacio.txt: no existe\n"

Evidencia en `target/self-improvement/tasks/T62/`.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límites.**

- El paso 4 —el tokenizador de `sosh`— va en ficha aparte,
  **[T64](T64-sosh-comillas.md)**: desde la shell sigue sin poderse *escribir*
  una ruta con espacios, aunque ya se pueda *pasar*. Es una decisión de
  sintaxis, no de transporte.
- El arreglo de `read_spawn_args` (fallback con `argv_ptr = 0`) **no lo
  ejercita nada en el árbol**; se deja por coherencia con `spawn_console`.
- Sincronizar `rootfs/src/soso/`, parado desde hacía meses, destapó que el
  manifiesto que Forja genera pedía `getrandom` con `rdrand` y `libsoso`
  necesita `custom`. Alineado. Una copia vendorizada sin sincronizar no es una
  copia antigua: es una segunda verdad que tapa los fallos de la primera.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). El fallo sólo existe dentro de soso —es su
paso de argumentos— y la sonda corre allí, entre dos procesos del guest.
Validación nativa: **verificada** (2026-09-24).
