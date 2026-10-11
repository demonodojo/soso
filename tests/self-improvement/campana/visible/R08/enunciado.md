# R08 — Entidades HTML tipográficas

## Qué hay hoy

`soso_web_core::decode::decode_entities` entiende `&amp; &lt; &gt; &quot; &apos; &nbsp;` y las
numéricas (`&#65;`, `&#x41;`). Las páginas reales usan también las tipográficas, y hoy salen
sin decodificar (`&hellip;` aparece literal en la pantalla).

## Qué se pide

Decodificar además:

| entidad | carácter |
|---|---|
| `&hellip;` | `…` (U+2026) |
| `&mdash;` | `—` (U+2014) |
| `&ndash;` | `–` (U+2013) |
| `&copy;` | `©` (U+00A9) |
| `&reg;` | `®` (U+00AE) |
| `&euro;` | `€` (U+20AC) |

Lo demás no cambia: una entidad **desconocida** (`&foo;`) se deja tal cual, y no se decodifica
dos veces (`&amp;hellip;` da `&hellip;`, no `…`).

## Alcance

Solo `crates/soso-web-core/src/decode.rs`, función `parse_entity_char`.

## Comprobación

Ejecuta `cargo test -p soso-web-core` antes y después de tu cambio y resume el resultado.
