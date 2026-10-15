# R22 — `ModelBundle::validate` no mira que los tensores cuadren con su forma

## Qué hay hoy
`ModelBundle::validate` (en `sosomodel`) comprueba que cada tensor del índice apunta a un shard
existente, que el shard pasa su CRC y que `offset + byte_len` cabe en el payload; y que
`prefetch` tiene una entrada por capa. No mira que `byte_len` sea el tamaño que le corresponde a
la `shape` y al `dtype` del tensor. Un índice con `byte_len` de más o de menos (o con un `dtype`
inventado) pasa la validación y el lector acaba leyendo bytes del tensor vecino sin dar error.
Tampoco rechaza dos tensores con el mismo nombre, y `TensorIndex::find` devolvería siempre el
primero.

## Qué se pide
`validate` debe devolver `Err(())` además en estos casos:

- el `byte_len` de una entrada no es el esperado para su `dtype` y su `shape`. Con `elems` igual
  al producto de las dimensiones de `shape`:
  - `DTYPE_F32`: `elems * 4`;
  - `DTYPE_Q8_0`, `DTYPE_Q4_K` y `DTYPE_MXFP4`: `ceil(elems / elems_por_bloque) * bytes_por_bloque`,
    con las constantes de bloque de `layout.rs`. Es lo que ya calculan las funciones
    `make_*_entry` de `index.rs`, así que las entradas que fabrican siguen validando;
- el `dtype` no es ninguno de esos cuatro;
- el producto de la forma desborda `u64` (no debe hacer pánico);
- dos entradas del índice tienen el mismo `name`.

Todo lo que ya devuelve `Err(())` sigue igual, y lo que ya validaba (con tamaños correctos y
nombres distintos) sigue validando.

## Alcance
Solo `crates/sosomodel/src/validate.rs`.

## Comprobación
`cargo test -p sosomodel`, antes y después de tu cambio.
