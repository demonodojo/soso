# R01 — El texto no ASCII sobrevive a decode_entities

## Reproducción

En la base de este caso, `soso_web_core::decode::decode_entities` corrompe el
texto que no es ASCII:

```rust
assert_eq!(decode_entities("café ☕"), "café ☕");
// izquierda:  "cafÃ© â\u{98}\u{95}"
```

Como `reflow_html` pasa por ahí todo el texto de una página, el navegador de
soso enseña mojibake en cuanto aparece una tilde. Evidencia de la
reproducción: `target/self-improvement/tasks/T02/repro-R01.log`.

## Qué se pide

Que `decode_entities` devuelva el texto intacto, sin perder lo que ya hace:

- las entidades con nombre (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&nbsp;`)
  y las numéricas (`&#233;`, `&#x2615;`) se siguen resolviendo;
- lo que no es una entidad conocida se queda literal, sin comerse el `&`;
- el texto que no es ASCII llega igual que entró.

`bytes_to_text` con `iso-8859-1` **sí** debe seguir tratando cada byte como un
carácter: eso no es un fallo, es lo que significa esa codificación.

## Alcance

Solo `crates/soso-web-core/src/decode.rs`. No cambies la firma pública.
