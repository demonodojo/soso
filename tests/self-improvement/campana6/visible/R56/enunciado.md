# R56 — soso-web-core: Clippy sin avisos

## Qué hay hoy
`cargo clippy` marca avisos en `soso-web-core` (código que compila y funciona, pero que no pasa la guía de estilo del proyecto). Algunos de los avisos que hoy hay en los archivos de esta tarea (ejecuta `cargo clippy` para ver la lista completa):

- `crates/soso-web-core/src/decode.rs:23`: this `if` statement can be collapsed
- `crates/soso-web-core/src/decode.rs:37`: this `if` statement can be collapsed
- `crates/soso-web-core/src/html.rs:254`: this `if` statement can be collapsed

## Qué se pide
Dejar `cargo clippy -p soso-web-core -- -D warnings -A clippy::result_unit_err` **sin avisos ni errores**, arreglando el código de verdad:

- sin `#[allow(...)]` ni `#![allow(...)]` ni cambios de configuración de Clippy para silenciarlos;
- el comportamiento no cambia: mismas funciones públicas con la misma firma, mismos resultados y errores, mismas pruebas pasando;
- Los avisos `result_unit_err` (`Result<_, ()>` en una función pública) **no se tocan**: arreglarlos obligaría a cambiar la firma pública. El criterio los permite (`-A clippy::result_unit_err`).
- los cambios van sólo en los archivos del alcance; lo que arregle Clippy en otro sitio no cuenta.

## Alcance
- `crates/soso-web-core/src/decode.rs`
- `crates/soso-web-core/src/html.rs`

## Comprobación
`cargo clippy -p soso-web-core -- -D warnings -A clippy::result_unit_err`, y después `cargo test -p soso-web-core`: todas las pruebas deben seguir en verde.
