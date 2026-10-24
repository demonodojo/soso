# R42 — gptdisk: una GPT corrupta o truncada debe dar error, no pánico ni una cabecera «válida»

## Qué hay hoy
`soso-install` lee la cabecera GPT y las entradas de un disco ajeno, que puede estar a medio escribir o
corrupto. Hoy `Header::parse` da por buena una cabecera con el CRC roto o con campos incoherentes, y varias
funciones de acceso a entradas entran en pánico (o desbordan) con índices o trozos de búfer extremos.

## Qué se pide
Sin cambiar ninguna firma ni añadir variantes a `Error`:

1. `Header::parse`: tras las comprobaciones que ya hace (firma, `header_size`), el CRC32 de la cabecera
   debe cuadrar. Se calcula sobre los primeros `header_size` bytes del sector con el campo CRC (bytes
   16..20) a cero, y se compara con el valor almacenado; si no coincide, `Err(Error::BadHeader)`. Una firma
   incorrecta sigue dando `BadSignature`, aunque el CRC también sea malo.
2. `Header::parse`: `entry_size` válido es una potencia de dos entre 128 y 4096 (ambos incluidos); `136`,
   `192`, `64` o `8192` dan `Err(Error::BadHeader)`. El rango de `num_entries` (1..=512) no cambia.
3. `Header::parse`: `my_lba == 0`, `entries_lba == 0` o `first_usable > last_usable` dan
   `Err(Error::BadHeader)`. `first_usable == last_usable` sigue siendo válido.
4. `entry` y `entry_mut`: si el índice es tan grande que `i * entry_size` o `i * entry_size + entry_size`
   desbordaría `usize`, devuelven `None` en vez de entrar en pánico. La última entrada que cabe en el búfer
   sigue devolviendo `Some`; la siguiente, `None`.
5. Acceso a una entrada más corta de lo debido (un trozo de búfer truncado), sin pánico:
   - `entry_used` da `false` si el trozo tiene menos de 16 bytes;
   - `entry_type` da `Guid::ZERO` si tiene menos de 16 bytes, y `entry_unique` si tiene menos de 32;
   - `entry_first_lba` da `0` si tiene menos de 40 bytes, y `entry_last_lba` si tiene menos de 48;
   - `set_entry_first_lba` (menos de 40 bytes) y `set_entry_last_lba` (menos de 48) no hacen nada.
   Con trozos suficientemente largos, todo funciona como hoy.
6. `relayout`: si alguna partición usada tiene `first_lba > last_lba`, devuelve `Err(Error::BadHeader)`
   sin modificar ni la cabecera ni las entradas. Esta comprobación va justo después de la de
   `ShortEntries` y antes de cualquier otra (incluida `TooSmall`). `first_lba == last_lba` es válido.

## Alcance
Solo `crates/gptdisk/src/lib.rs`.

## Comprobación
`cargo test -p gptdisk`, antes y después de tu cambio.
