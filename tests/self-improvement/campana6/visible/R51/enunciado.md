# R51 — block-dev: Clippy sin avisos

## Qué hay hoy
`cargo clippy` marca avisos en `block-dev` (código que compila y funciona, pero que no pasa la guía de estilo del proyecto). Algunos de los avisos que hoy hay en los archivos de esta tarea (ejecuta `cargo clippy` para ver la lista completa):

- `crates/block-dev/src/lib.rs:74`: manual implementation of `.is_multiple_of()`
- `crates/block-dev/src/lib.rs:77`: using `chunks_exact_mut` with a constant chunk size
- `crates/block-dev/src/lib.rs:154`: manual implementation of `.is_multiple_of()`

## Qué se pide
Dejar `cargo clippy -p block-dev --features std -- -D warnings -A clippy::result_unit_err` **sin avisos ni errores**, arreglando el código de verdad:

- sin `#[allow(...)]` ni `#![allow(...)]` ni cambios de configuración de Clippy para silenciarlos;
- el comportamiento no cambia: mismas funciones públicas con la misma firma, mismos resultados y errores, mismas pruebas pasando;
- los cambios van sólo en los archivos del alcance; lo que arregle Clippy en otro sitio no cuenta.

## Alcance
- `crates/block-dev/src/lib.rs`

## Comprobación
`cargo clippy -p block-dev --features std -- -D warnings -A clippy::result_unit_err`, y después `cargo test -p block-dev --features std`: todas las pruebas deben seguir en verde.
