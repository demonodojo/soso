# R54 — soso-log-core y soso-red-core: Clippy sin avisos

## Qué hay hoy
`cargo clippy` marca avisos en `soso-log-core` y `soso-red-core` (código que compila y funciona, pero que no pasa la guía de estilo del proyecto). Algunos de los avisos que hoy hay en los archivos de esta tarea (ejecuta `cargo clippy` para ver la lista completa):

- `crates/soso-log-core/src/texto.rs:56`: using `write!()` with a format string that ends in a single newline
- `crates/soso-log-core/src/texto.rs:64`: using `write!()` with a format string that ends in a single newline
- `crates/soso-log-core/src/texto.rs:72`: using `write!()` with a format string that ends in a single newline
- `crates/soso-red-core/src/estatica.rs:145`: using `write!()` with a format string that ends in a single newline

## Qué se pide
Dejar `cargo clippy -p soso-log-core --features std -- -D warnings -A clippy::result_unit_err` ; `cargo clippy -p soso-red-core --features std -- -D warnings -A clippy::result_unit_err` **sin avisos ni errores**, arreglando el código de verdad:

- sin `#[allow(...)]` ni `#![allow(...)]` ni cambios de configuración de Clippy para silenciarlos;
- el comportamiento no cambia: mismas funciones públicas con la misma firma, mismos resultados y errores, mismas pruebas pasando;
- los cambios van sólo en los archivos del alcance; lo que arregle Clippy en otro sitio no cuenta.

## Alcance
- `crates/soso-log-core/src/texto.rs`
- `crates/soso-red-core/src/estatica.rs`

## Comprobación
`cargo clippy -p soso-log-core --features std -- -D warnings -A clippy::result_unit_err` ; `cargo clippy -p soso-red-core --features std -- -D warnings -A clippy::result_unit_err`, y después `cargo test -p soso-log-core --features std` ; `cargo test -p soso-red-core --features std`: todas las pruebas deben seguir en verde.
