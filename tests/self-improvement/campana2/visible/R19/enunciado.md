# R19 — `last_used` y los empates

## Qué hay hoy
`gptdisk::last_used(entries, hdr)` devuelve el índice de la entrada usada cuya partición **acaba
más tarde** (`last_lba` mayor). Cuando dos entradas acaban en el **mismo** LBA, la comparación
`>=` se queda con la **última** que recorre.

## Qué se pide
Un empate debe resolverse a favor de la entrada de **menor índice** (la que aparece primero en la
tabla). Todo lo demás igual: se ignoran las entradas sin tipo (`entry_used` falso), una tabla
vacía da `None`, y con `last_lba` distintos gana el mayor aunque tenga mayor índice.

## Alcance
Solo `crates/gptdisk/src/lib.rs`, función `last_used`.

## Comprobación
`cargo test -p gptdisk`, antes y después de tu cambio.
