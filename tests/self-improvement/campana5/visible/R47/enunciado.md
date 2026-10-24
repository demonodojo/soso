# R47 — `read_file_range` recorre los extents de todo el fichero

## Qué hay hoy
En `sosofs`, `Sosofs::read_file_range(ino, offset, len, out)` lee un trozo de un fichero. Para localizar
los datos pide al árbol **todos** los extents del inode (`Key::range(ino, KIND_EXTENT)`), aunque sólo
unos pocos solapen con el rango pedido. En un fichero grande, leer 100 bytes del final cuesta
tantas lecturas de bloque de metadatos como hojas del árbol ocupen sus extents, es decir, crece con el
tamaño del fichero en vez de con el del rango.

## Qué se pide
Que `read_file_range` consulte sólo los extents que pueden solaparse con `[offset, offset + len)`,
con **el mismo resultado observable** que hoy:

- el contenido devuelto es idéntico;
- se conservan los errores y su prioridad: `Corrupt` si `out` es más corto que `len` o si
  `offset + len` excede el tamaño del fichero, `NotAFile` si el inode no es un fichero, `BadChecksum`
  si el crc de un extent leído no cuadra, `Ok` sin leer datos si `len == 0`;
- cada extent se sigue verificando entero con su crc antes de copiar de él.

Dato útil: la clave de un extent es el offset de fichero donde empieza, y ningún extent ocupa más de
`EXTENT_MAX_BLOCKS` bloques (32, es decir 128 KiB).

**Límite medible.** La prueba cuenta **cada bloque leído** del dispositivo (no peticiones) durante una
sola llamada a `read_file_range`, sobre un fichero de 48 MiB hecho de extents de exactamente 32 bloques.
Con `E` = número de extents que el rango toca realmente (los que se solapan con él; uno que termina
justo donde empieza el rango no cuenta), la llamada debe leer como mucho `8 + 32·E` bloques
(los `32·E` son los datos de esos extents; los 8 son todos los metadatos del árbol y del inode).
Hoy gasta unos 70 bloques para `E = 1`.

## Alcance
Solo `crates/sosofs/src/fs.rs`, función `read_file_range`. No cambies firmas públicas ni el formato en
disco.

## Comprobación
`cargo test -p sosofs --features std`, antes y después de tu cambio.
