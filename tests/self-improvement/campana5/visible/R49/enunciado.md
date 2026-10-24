# R49 — Tokenizador: decodificación en streaming, espacio inicial GPT-2, piezas de byte y ids fuera de rango

## Qué hay hoy
`crates/soso-llm-core/src/tokenizer.rs` convierte ids de token en texto (`Tokenizer::decode`,
`Tokenizer::token_bytes` y el decodificador incremental `StreamDecoder`, que usa el bucle de generación
para ir imprimiendo). Hay cuatro síntomas:

- **Streaming con bytes inválidos.** Con el tokenizador byte-level, `StreamDecoder::push` con el id `0xFF`
  devuelve `""` y deja el byte retenido; si siguen `b'a'`, `b'b'`, ... no sale nada hasta que se acumulan
  más de 4 bytes y entonces se descarta todo, **también el texto válido** que venía detrás. Lo mismo pasa
  con `0xE2` seguido de `b'a'`: nunca se emite la `a`. Concatenar todo lo que devuelve el decodificador
  no coincide con `String::from_utf8_lossy` de los mismos bytes.
- **Espacio inicial en vocabularios GPT-2/Qwen2.** `decode(&[id_de("Ġmundo")])` devuelve `"mundo"` en
  lugar de `" mundo"`, así que `decode(encode(" hola"))` pierde el espacio. (En SentencePiece el
  espacio inicial sí es de cortesía y se quita.)
- **Piezas de byte.** Una pieza `<0x+1>` se interpreta como el byte `0x01`, cuando sólo debería serlo
  `<0xXX>` con dos dígitos hexadecimales de verdad.
- **Byte-level con ids grandes.** `decode(&[0x141])` devuelve `"A"`: el id se trunca a `u8` en vez de
  ignorarse.

## Qué se pide
Corregir `tokenizer.rs` para que se cumpla todo esto:

1. `StreamDecoder::push` acumula los bytes del token y, en cada llamada, emite todo lo que ya se puede
   decidir: el texto válido, y **un `U+FFFD` por cada secuencia de bytes inválida** (la misma segmentación
   que `String::from_utf8_lossy`) en cuanto se sabe que es inválida, seguido de lo que haya detrás. Un
   prefijo UTF-8 incompleto se retiene a la espera del siguiente token. Ejemplos con byte-level:
   `[0xFF, b'a']` da `["\u{FFFD}", "a"]`; `[0xE2, b'a']` da `["", "\u{FFFD}a"]`;
   `[0xE2, 0x82, b'a']` da `["", "", "\u{FFFD}a"]`. Propiedad general: la concatenación de todos los
   `push` más `finish()` es igual a `String::from_utf8_lossy` de la secuencia de bytes completa. `finish()`
   sigue devolviendo un único `U+FFFD` por un prefijo incompleto pendiente y deja el decodificador vacío.
2. `VocabTokenizer::decode` (y por tanto `Tokenizer::decode`) quita **un** espacio inicial sólo si el
   vocabulario es de tipo SentencePiece (marca `▁`); en un vocabulario GPT-2/Qwen2 (marca `Ġ`) el
   espacio inicial se conserva. Con SentencePiece el comportamiento actual no cambia.
3. Una pieza sólo es de byte-fallback si es exactamente `<0x`, dos dígitos hexadecimales
   (`0-9`, `A-F` o `a-f`, sin signo ni otro carácter) y `>`. Cualquier otra, como `<0x+1>`, es una pieza de
   texto normal y se emite tal cual.
4. Con el tokenizador byte-level, un id `>= 256` no es un byte: `decode`, `token_bytes` y
   `StreamDecoder::push` no emiten nada para él (los ids 0–255 siguen siendo ese byte).

## Alcance
Solo `crates/soso-llm-core/src/tokenizer.rs`. No cambies firmas públicas.

## Comprobación
`cargo test -p soso-llm-core --features std`, antes y después de tu cambio.
