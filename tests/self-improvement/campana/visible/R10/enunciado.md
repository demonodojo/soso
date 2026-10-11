# R10 — `Span::contains` y `Span::overlaps`

## Qué hay hoy

`soso_update_core::plan::Span` representa un tramo `[start, end)` del pack, pero para saber si
un desplazamiento cae dentro de él, o si dos tramos se pisan, cada llamante repite a mano la
comparación (y se equivoca con los extremos).

## Qué se pide

Añadir a `Span`:

```rust
pub fn contains(&self, offset: u64) -> bool
pub fn overlaps(&self, other: &Span) -> bool
```

con la semántica del intervalo **semiabierto** `[start, end)`:

- `contains(offset)` es cierto si `start <= offset < end` (el final **no** cuenta);
- `overlaps` es cierto si comparten **al menos un byte**;
- dos tramos que sólo se tocan (uno acaba donde empieza el otro) **no** se solapan;
- un tramo **vacío** no contiene nada y no se solapa con ninguno;
- `overlaps` es simétrico.

El crate es `no_std + alloc`: nada de `std`.

## Alcance

Solo `crates/soso-update-core/src/plan.rs`.

## Comprobación

Ejecuta `cargo test -p soso-update-core --features std` antes y después de tu cambio y resume el resultado.
