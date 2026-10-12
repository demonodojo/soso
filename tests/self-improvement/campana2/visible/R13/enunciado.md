# R13 — `PartGeom`: geometría coherente

## Qué hay hoy
`PartGeom::from_lba(first, last)` calcula `sectors = last.saturating_sub(first) + 1`: con
`last < first` (un hueco invertido) da **1** sector, como si la partición existiera.

## Qué se pide
En `PartGeom`:
- `from_lba(first, last)` con `last < first` da `sectors == 0` (el resto igual; con `last >= first`
  sigue siendo `last - first + 1`);
- `pub fn contains(&self, lba: u64) -> bool`: `first <= lba <= last` (ambos extremos **incluidos**),
  y falso siempre si `sectors == 0`;
- `pub fn overlaps(&self, other: &PartGeom) -> bool`: comparten al menos un sector (rangos
  cerrados); una geometría con 0 sectores no se solapa con nada; simétrico.

## Alcance
Solo `crates/soso-resize-core/src/lib.rs`, `impl PartGeom`.

## Comprobación
`cargo test -p soso-resize-core`, antes y después de tu cambio.
