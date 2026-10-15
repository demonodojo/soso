# R26 — `GeomCache::reload` fallido deja la caché a medias

## Qué hay hoy
`GeomCache` guarda en memoria la geometría de las particiones GPT (raíz, modelos, ESP, install).
`reload(&disk)` las vuelve a leer del disco y las va escribiendo en la caché una a una. Si la lectura
de una partición obligatoria falla a mitad (error de E/S en el disco), `reload` devuelve `Err`, pero
para entonces la caché ya contiene parte de la geometría nueva y parte de la vieja: por ejemplo la
raíz con los límites posteriores a un resize y los modelos con los anteriores. Quien siga consultando
`get` después del error ve un disco que nunca existió.

## Qué se pide
`reload` debe ser **todo o nada** respecto a su resultado:
- Si devuelve `Err`, la caché queda exactamente como estaba antes de la llamada: mismas respuestas de
  `get` para cualquier índice y el mismo valor de `reloads()` (un reload fallido no cuenta).
- Si devuelve `Ok`, la caché tiene la geometría recién leída y `reloads()` sube en uno.

No cambia qué particiones son obligatorias (raíz y modelos) y cuáles opcionales (ESP e install: si no
se pueden leer, quedan en `None` y el reload sigue siendo `Ok`), ni el tipo de error que devuelve
`reload`, ni la API pública.

## Alcance
Solo `crates/soso-resize-core/src/lib.rs`, `GeomCache::reload`.

## Comprobación
`cargo test -p soso-resize-core`, antes y después de tu cambio.
