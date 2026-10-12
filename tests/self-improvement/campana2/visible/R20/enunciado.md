# R20 — `SemVer`: texto y máximo

## Qué hay hoy
`soso_update_core::semver::SemVer` se parsea (`parse`) y se compara (`cmp`), pero no se puede
**escribir** (hay que formatear los tres campos a mano) ni elegir la mayor de una lista.

## Qué se pide
En `semver.rs`:
- `impl core::fmt::Display for SemVer`: `major.minor.patch` sin ceros a la izquierda
  (`SemVer{0,10,3}` → `0.10.3`); el texto vuelve a parsearse al mismo valor;
- `pub fn max(versions: &[SemVer]) -> Option<&SemVer>`: la mayor según `cmp`; `None` si la lista
  está vacía; si hay varias iguales y máximas, devuelve **la primera** de ellas.

El crate es `no_std + alloc`: nada de `std`.

## Alcance
Solo `crates/soso-update-core/src/semver.rs`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.
