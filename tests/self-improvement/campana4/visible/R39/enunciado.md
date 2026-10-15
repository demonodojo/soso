# R39 — `MemBlockDevice`: un número de bloque enorme es `OutOfRange`, no un pánico

## Qué hay hoy
`MemBlockDevice` (en `block-dev`) es el dispositivo en memoria que usan los tests y la
construcción de imágenes. Su contrato es que un acceso fuera de la imagen devuelve
`Err(BlockError::OutOfRange)`. Eso se cumple para números de bloque pequeños, pero los tres
métodos que calculan el desplazamiento en bytes (`read_block`, `write_block` y `read_blocks`)
lo hacen con aritmética sin comprobar: con un número de bloque muy grande (por ejemplo, el que
sale de un campo corrupto de una imagen, o `u64::MAX`) la multiplicación por `BLOCK_SIZE` o la
suma de la longitud desbordan, y en compilación de depuración el proceso entra en pánico en
lugar de devolver el error. En release el desbordamiento da la vuelta y puede llegar a leer o
escribir en un bloque que no es el pedido.

## Qué se pide
Que `read_block`, `write_block` y `read_blocks` de `MemBlockDevice` devuelvan
`Err(BlockError::OutOfRange)` para **cualquier** `u64` que quede fuera de la imagen, incluidos
los que desbordan el cálculo del desplazamiento o de su final. Un acceso fallido no modifica ni
la imagen ni el búfer del llamante. El comportamiento con números válidos, la comprobación de
que la longitud de `read_blocks` sea múltiplo de `BLOCK_SIZE` y lo que cuenta `read_count()`
(una petición de `read_blocks` = una lectura) no cambian.

## Alcance
Solo `crates/block-dev/src/lib.rs`, `MemBlockDevice`. No cambia la API pública.

## Comprobación
`cargo test -p block-dev --features std`, antes y después de tu cambio.
