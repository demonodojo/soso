# R05 — Trocear un tramo de descarga que no cabe en RAM

## Qué hay hoy

`soso_update_core::plan::Span::oversized` dice si un tramo se pasa de
`span_max` —pasa con un fichero suelto más grande que el tope, y `MAX_FILE_SIZE`
son 64 MiB frente a un `SPAN_MAX` de 8 MiB—, pero no hay forma de trocearlo:
quien lo detecte sigue teniendo que pedir el tramo entero.

## Qué se pide

Añadir a `Span`:

```rust
pub fn split(&self, span_max: u64) -> Vec<Span>
```

con este comportamiento:

- devuelve trozos **contiguos, sin huecos ni solapes**, que cubren exactamente
  `[start, end)`;
- ninguno pasa de `span_max` bytes y ninguno queda vacío;
- cada trozo conserva la lista `files` del tramo original;
- un tramo que ya cabe se devuelve tal cual, en un vector de un elemento;
- `span_max` a 0 no trocea: devuelve el tramo entero (no se cuelga).

El crate es `no_std + alloc`: nada de `std`.

## Alcance

Solo `crates/soso-update-core/src/plan.rs`.
