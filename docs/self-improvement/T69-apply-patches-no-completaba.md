# T69 — `dl.rs` se copia al vendor y nadie lo compila

**Origen:** medido en [T39](T39-bootstrap-libstd.md), fuera de su alcance.
**Aplica en:** `config/rust-soso/` · **Estado:** completada (2026-09-26).

## Lo que se ve

`apply-patches.sh` copia dos cosas a `library/std/src/sys/pal/soso/`:

1. El árbol `config/rust-soso/tree/.../sys/pal/soso/`, cuyo `mod.rs` **no
   declara submódulos** («PAL soso — basada en `unsupported` + `soso_rt`»).
2. Encima, `config/rust-soso/sys/pal/soso/dl.rs`.

El `mod.rs` que queda no dice `mod dl;`, así que ese fichero está en el vendor
y **nada lo compila**. Comprobado en el vendor real:

    $ ls ~/.cache/soso-rust-vendor/library/std/src/sys/pal/soso/
    dl.rs  mod.rs
    $ grep -n "mod dl" .../soso/mod.rs
    (nada)

## De dónde viene

Hay **dos generaciones** de la PAL en el repo:

- `config/rust-soso/sys/pal/soso/` — la antigua. Su `mod.rs` declara
  `pub mod dl; pub mod io; pub mod thread;` y su comentario dice «Copiar este
  árbol a `library/std/src/sys/pal/soso/`».
- `config/rust-soso/tree/library/std/src/sys/pal/soso/` — la nueva, basada en
  `unsupported`, sin submódulos.

El script copia la **nueva** y arrastra un `cp` de `dl.rs` de la **antigua**.
De la antigua, `io.rs` y `thread.rs` no se copian nunca.

## Qué hay que decidir

Una de dos, y hace falta compilar para saber cuál:

1. La PAL nueva **necesita** `dl`, y entonces su `mod.rs` debe declararlo (y
   quizá `io` y `thread` también).
2. No lo necesita, y entonces sobran el `cp` y los tres ficheros de la
   generación antigua.

No se decide aquí porque **hoy no se ha compilado libstd para soso**. El
enlazador ya está (`wild` 0.10.0, decisión D3); lo que falta es el build de
[T39](T39-bootstrap-libstd.md). Decidirlo a ciegas es justo lo que hizo que
dos generaciones convivieran.

## 2026-09-25 · ya se ha compilado, y la respuesta apunta a la opción 2

La ficha decía que decidir necesitaba compilar libstd. Se compiló
(ver [seguimiento/T39.md](seguimiento/T39.md)) y `std` falló con 13 errores en
seis huecos. **Ninguno menciona `dl`.** Lo que `std` pide de la PAL es otra
cosa: `sys::io` (`errno`, `decode_error_kind`, `format_error`,
`is_interrupted`), `sys::thread_local::key` (`Key`, `LazyKey`, `get`, `set`),
ramas de `cfg_select` en `sys/alloc` y `sys/io/error`, y sincronización de
verdad en vez de `no_threads`.

Es decir: de la generación antigua lo que hacía falta no era `dl.rs` —el único
que el script sí copia— sino `io.rs` y `thread.rs`, **que no copia nunca**. Y
ni siquiera tal cual: en el std de hoy esas piezas viven en `sys/io/error` y
`sys/thread_local/key`, no en `pal/soso/`.

**Cautela:** que `dl` no aparezca ahora no prueba que no haga falta nunca;
prueba que no es lo que bloquea. La compilación se paró en esos seis huecos y
no llegó más lejos. La ficha se podrá cerrar cuando `std` compile: si entonces
`dl` sigue sin aparecer, sobra el `cp` y sobra la generación antigua.

## Mientras tanto

`soso_improve_core::receta::pasos_libstd` traduce el script **fielmente**,
copia incluida, y el paso se llama «PAL: dl.rs (hoy no lo declara nadie —
T69)». Una traducción que arregla cosas por el camino deja de ser una
traducción.

## Revisión del 26-sep-2026

Estado y siguiente paso sincronizados con el catálogo; ver
[seguimiento/T69.md](seguimiento/T69.md) y
[revisión del plan](seguimiento/REVISION-2026-09-26.md).

La equivalencia histórica entre receta y script ya no es vigente: T75 la
restaura. `dl.rs` sigue copiado por ambas rutas; retirar una sola no cierra T69.

## Cierre (2026-09-26)

Libstd de T39 enlaza y el humo guest imprime `std-soso-ok` sin que `mod.rs` declare `dl`. Opción 2: se retira el `cp`, el paso de la receta y la generación antigua (`config/rust-soso/sys/pal/soso/`). El espejo borra un `dl.rs` que quede en el vendor.

Comprobación: `cargo test -p soso-improve-core --features std receta --lib` 15/15; `cargo test -p soso-improve --test receta` 2/2; `cargo xtask rust-build-std` exit 0 y el directorio PAL del vendor queda sólo con `mod.rs`. Ver [seguimiento/T69.md](seguimiento/T69.md).
