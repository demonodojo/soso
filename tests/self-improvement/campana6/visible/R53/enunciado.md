# R53 — soso-http: Clippy sin avisos

## Qué hay hoy
`cargo clippy` marca avisos en `soso-http` (código que compila y funciona, pero que no pasa la guía de estilo del proyecto). Algunos de los avisos que hoy hay en los archivos de esta tarea (ejecuta `cargo clippy` para ver la lista completa):

- `crates/soso-http/src/lib.rs:58`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:190`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:202`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:230`: this expression creates a reference which is immediately dereferenced by the compiler
- `crates/soso-http/src/lib.rs:271`: this expression creates a reference which is immediately dereferenced by the compiler
- `crates/soso-http/src/lib.rs:282`: call to `std::mem::drop` with a value that does not implement `Drop`. Dropping such a type only extends its contained lifetimes
- `crates/soso-http/src/lib.rs:321`: this expression creates a reference which is immediately dereferenced by the compiler
- `crates/soso-http/src/lib.rs:361`: this `if` can be collapsed into the outer `match`
- `crates/soso-http/src/lib.rs:399`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:408`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:455`: this `if` statement can be collapsed
- `crates/soso-http/src/lib.rs:478`: this `if` statement can be collapsed
- `crates/soso-http/src/lib.rs:479`: this `if` statement can be collapsed
- `crates/soso-http/src/lib.rs:625`: casting to the same type is unnecessary (`i64` -> `i64`)
- `crates/soso-http/src/lib.rs:650`: this `let...else` may be rewritten with the `?` operator
- `crates/soso-http/src/lib.rs:809`: this `if` statement can be collapsed

## Qué se pide
Dejar `cargo clippy -p soso-http --features std -- -D warnings -A clippy::result_unit_err` **sin avisos ni errores**, arreglando el código de verdad:

- sin `#[allow(...)]` ni `#![allow(...)]` ni cambios de configuración de Clippy para silenciarlos;
- el comportamiento no cambia: mismas funciones públicas con la misma firma, mismos resultados y errores, mismas pruebas pasando;
- los cambios van sólo en los archivos del alcance; lo que arregle Clippy en otro sitio no cuenta.

## Alcance
- `crates/soso-http/src/lib.rs`

## Comprobación
`cargo clippy -p soso-http --features std -- -D warnings -A clippy::result_unit_err`, y después `cargo test -p soso-http --features std`: todas las pruebas deben seguir en verde.
