# R50 — `validate_geometry`: rechazar deltas y geometrías peligrosas con un error

## Qué hay hoy
`validate_geometry(disk, delta_sectors)` (en `soso-resize-core`) es la comprobación previa del
redimensionado en vivo: el kernel la llama antes de empezar, y sólo si devuelve `Ok(())` se pasa a
encoger modelos, deslizar p3 y p4 y reescribir la GPT. Lee de la GPT las particiones p2 (`GPT_ROOT`),
p3 (`GPT_MODELS`) y p4 (`GPT_INSTALL`); el resize hace crecer p2 `delta_sectors` por el final, desliza p3 y
p4 hacia delante esa misma cantidad y deja p3 con `delta_sectors` sectores menos (su último sector no se mueve).

Hoy sólo rechaza `delta >= sectores de p3` y lo que no cabe en el disco. Con otras entradas incoherentes
da `Ok` (y el plan resultante pisaría datos) o entra en pánico por sumas que desbordan `u64`.

## Qué se pide
Que `validate_geometry` devuelva `Err(Error::InvalidGeometry)` (sin pánico, en cualquiera de los casos) cuando:

1. `delta_sectors` es 0.
2. `delta_sectors` no es múltiplo de 8 (el redimensionado trabaja en bloques de 4 KiB = 8 sectores de 512 B).
3. Alguna de p2, p3 o p4 tiene su último sector menor que su primero.
4. Las particiones no están en orden estricto y sin solapes: se exige `p2.last < p3.first` y
   `p3.last < p4.first` (que dos particiones compartan un sector ya es un solape; que haya huecos entre
   ellas es válido).
5. Alguna suma del relayout (`p2.last + delta`, `p3.first + delta`, `p4.first + delta`, `p4.last + delta`)
   o el cálculo del tamaño de una partición desborda `u64`, incluida una partición `first = 0`,
   `last = u64::MAX`.
6. Se mantiene el rechazo actual de `delta >= sectores de p3` (con sectores de p3 = `last - first + 1`);
   p3 de 100 sectores admite `delta = 96`, no `104`.

Los casos válidos siguen dando `Ok(())` (por ejemplo p2 = 34..=999, p3 = 1000..=1099, p4 = 1100..=1149 con
delta 8, 16, 64 o 96; también con huecos entre particiones). Todos los rechazos usan el mismo error. Los valores de `delta` se comprueban todos
con el mismo error, sea cual sea el orden en que los compruebes.

## Alcance
Solo `crates/soso-resize-core/src/lib.rs` (`validate_geometry` y, si lo necesitas, el cálculo de
`PartGeom::from_lba`). No cambies firmas públicas ni el comportamiento de las demás funciones.

## Comprobación
`cargo test -p soso-resize-core --features std`, antes y después de tu cambio.
