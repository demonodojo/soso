# R52 — gptdisk: Clippy sin avisos

## Qué hay hoy
`cargo clippy` marca avisos en `gptdisk` (código que compila y funciona, pero que no pasa la guía de estilo del proyecto). Algunos de los avisos que hoy hay en los archivos de esta tarea (ejecuta `cargo clippy` para ver la lista completa):

- `crates/gptdisk/src/lib.rs:87`: the loop variable `i` is used to index `g`
- `crates/gptdisk/src/lib.rs:164`: manual implementation of `.is_multiple_of()`

## Qué se pide
Dejar `cargo clippy -p gptdisk -- -D warnings -A clippy::result_unit_err` **sin avisos ni errores**, arreglando el código de verdad:

- sin `#[allow(...)]` ni `#![allow(...)]` ni cambios de configuración de Clippy para silenciarlos;
- el comportamiento no cambia: mismas funciones públicas con la misma firma, mismos resultados y errores, mismas pruebas pasando;
- los cambios van sólo en los archivos del alcance; lo que arregle Clippy en otro sitio no cuenta.

## Alcance
- `crates/gptdisk/src/lib.rs`

## Comprobación
`cargo clippy -p gptdisk -- -D warnings -A clippy::result_unit_err`, y después `cargo test -p gptdisk`: todas las pruebas deben seguir en verde.
