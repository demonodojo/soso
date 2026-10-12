# R15 — `PackReader::verify_all`

## Qué hay hoy
`PackReader::verify_entry` comprueba **una** entrada (que esté dentro del pack y que su sha256
coincida). Quien quiere validar un pack entero tiene que escribir el bucle y suele quedarse con
el primer fallo.

## Qué se pide
Añadir a `PackReader`:

```rust
pub fn verify_all<'e>(&self, files: &'e [FileEntry]) -> Vec<&'e FileEntry>
```

- devuelve **todas** las entradas que **no** verifican (hash distinto, o que se salen del pack),
  **en el orden** en que vienen en `files`;
- si todas verifican (o `files` está vacío), devuelve un vector vacío;
- no modifica nada y usa `verify_entry`.

## Alcance
Solo `crates/soso-update-core/src/pack.rs`, `impl PackReader`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.
