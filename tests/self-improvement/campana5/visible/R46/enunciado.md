# R46 — Codificación y decodificación porcentual en `soso-web-core`

## Qué hay hoy
`soso-web-core` (crate `no_std` con `alloc`) tiene el parser HTML, `resolve_url` y las utilidades de
`decode`, pero ninguna función para codificar o decodificar el «porcentaje» de las URL (`%20`,
`%C3%A9`…). Quien necesita construir una query o leer un parámetro lo hace a mano.

## Qué se pide
Añadir en la **raíz del crate** (`crates/soso-web-core/src/lib.rs`), públicos y exportados como
`soso_web_core::…`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentError {
    /// Un `%` no va seguido de dos dígitos hexadecimales.
    BadEscape,
    /// Los bytes decodificados no forman UTF-8 válido.
    InvalidUtf8,
}

pub fn percent_encode_component(s: &str) -> String;
pub fn percent_decode(s: &str) -> Result<String, PercentError>;
```

1. `percent_encode_component` trabaja byte a byte sobre el UTF-8 de `s`: los bytes «no reservados»
   (`A-Z`, `a-z`, `0-9`, `-`, `.`, `_`, `~`) se copian tal cual y **cualquier otro byte** se escribe
   como `%` más dos dígitos hexadecimales en **mayúsculas**. Ejemplos: `"a b"` → `"a%20b"`,
   `"é"` → `"%C3%A9"`, `"a/b?c=d&e"` → `"a%2Fb%3Fc%3Dd%26e"`, `"%"` → `"%25"`, `""` → `""`.
2. `percent_decode` convierte cada `%XX` (dígitos hexadecimales en mayúsculas o minúsculas) en el
   byte correspondiente y copia el resto de bytes sin tocarlos. En particular `+` **no** se
   convierte en espacio. Después interpreta el resultado como UTF-8: `"%C3%A9"` → `"é"`,
   `"a%20b"` → `"a b"`, `"%41%62"` → `"Ab"`, `"%2b"` → `"+"`, `"a+b"` → `"a+b"`, `"%00"` → `"\0"`.
   Un `%` seguido de menos de dos caracteres, o de algún carácter que no sea dígito hexadecimal
   (`"%"`, `"%4"`, `"%zz"`, `"100%"`, `"%4g"`), da `Err(PercentError::BadEscape)`.
3. Si hay a la vez un escape inválido y bytes que no son UTF-8 válido, gana `BadEscape`
   (se valida toda la cadena antes de comprobar el UTF-8). Si todos los escapes son válidos pero el
   resultado no es UTF-8 (`"%FF"`, `"%C3"`, `"a%E9b"`) el error es `Err(PercentError::InvalidUtf8)`.
4. Para toda `s: &str`, `percent_decode(&percent_encode_component(s)) == Ok(s.to_string())`.

## Alcance
Sólo `crates/soso-web-core/src/lib.rs`. No toques `decode.rs` ni `html.rs`, y no cambies ninguna
firma pública existente.

## Comprobación
`cargo test -q -p soso-web-core`, antes y después de tu cambio.
