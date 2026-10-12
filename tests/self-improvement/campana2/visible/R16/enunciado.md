# R16 — Entidades numéricas inválidas

## Qué hay hoy
`decode_entities` decodifica `&#65;` y `&#x41;`. Si el número **no es un carácter válido**
(`&#0;`, `&#xD800;` —un sustituto—, `&#1114112;` —pasa de U+10FFFF—), `char::from_u32` da `None` y
la entidad **desaparece sin dejar rastro**: el texto pierde un trozo y nadie lo nota.

## Qué se pide
Siguiendo el estándar de HTML, una referencia numérica **bien formada pero inválida** (cero,
sustituto o fuera de rango) se sustituye por el carácter de reemplazo `U+FFFD` (`�`). Lo demás no
cambia: las válidas se decodifican, y una entidad **mal formada** (`&#xZZ;`, `&#;`, o un número
que no cabe en `u32`) se deja tal cual, como hasta ahora.

## Alcance
Solo `crates/soso-web-core/src/decode.rs`, función `parse_entity_char`.

## Comprobación
`cargo test -p soso-web-core`, antes y después de tu cambio.
