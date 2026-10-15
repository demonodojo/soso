# R31 — `Catalog::parse` no debe entrar en pánico con datos truncados o corruptos

## Qué hay hoy
`Catalog::parse(data)` (en `sosomfs`, módulo `catalog`) reconstruye el catálogo de modelos a partir
de los bytes leídos del disco. Devuelve `Result<Catalog, ()>`, pero sólo comprueba el tamaño de
forma aproximada, así que con un buffer cortado o con un contador corrupto no devuelve `Err`:

- hay longitudes mínimas que se quedan cortas respecto a lo que la función lee a continuación
  (la cabecera de cada modelo y la de cada shard), y indexar más allá del final provoca un pánico;
- un shard puede quedar cortado justo después de su lista de extents y la parte final fija
  (prefetch, política de caché, flags, crc) se lee sin comprobar que quede espacio;
- los contadores de modelos y de shards se usan tal cual para reservar memoria
  (`Vec::with_capacity`), de modo que un valor como `u32::MAX` en un catálogo dañado intenta
  reservar decenas de gigabytes antes de fallar.

En un catálogo dañado el kernel debe poder rechazar el volumen, no caerse.

## Qué se pide
Que `Catalog::parse` devuelva `Err(())` (sin pánico y sin reservas desproporcionadas) para
**cualquier** entrada que no contenga el catálogo completo que anuncia, y en particular:

- cualquier prefijo estricto de un catálogo bien serializado debe dar `Err`;
- un contador de modelos o de shards imposible de satisfacer con los bytes disponibles debe dar
  `Err` antes de reservar nada;
- un contador de extents que promete más datos de los que quedan debe dar `Err`.

Las entradas válidas deben seguir dando el mismo resultado que hoy, incluido un buffer con ceros de
relleno al final (así llega desde la lectura por bloques) y el catálogo vacío.

## Alcance
Solo `crates/sosomfs/src/catalog.rs` (las funciones de lectura de `Catalog::parse` y sus
auxiliares). No cambia el formato en disco ni la firma pública.

## Comprobación
`cargo test -p sosomfs`, antes y después de tu cambio.
