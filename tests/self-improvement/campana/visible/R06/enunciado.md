# R06 — `header_values`: todas las cabeceras repetidas

## Qué hay hoy

`soso_http::header_value` devuelve sólo la **primera** cabecera con ese nombre. Hay
cabeceras que se repiten legítimamente (`Set-Cookie`, `Link`, `Via`) y hoy no hay
forma de verlas todas.

## Qué se pide

Añadir, junto a `header_value`, la función pública:

```rust
pub fn header_values<'a>(headers: &'a [(String, String)], key: &str) -> Vec<&'a str>
```

- devuelve **todos** los valores de las cabeceras llamadas `key`, **sin distinguir
  mayúsculas**, en el **orden en que aparecen**;
- si no hay ninguna, devuelve un vector vacío;
- no cambia `header_value` ni el resto del comportamiento.

## Alcance

Solo `crates/soso-http/src/lib.rs`. El archivo es largo: localiza `header_value` con `grep` y
lee sólo ese tramo con `read`.

## Comprobación

Ejecuta `cargo test -p soso-http` antes y después de tu cambio y resume el resultado.
