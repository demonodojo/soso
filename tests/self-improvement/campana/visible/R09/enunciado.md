# R09 — `resolve_url` con `#fragmento` y `?consulta`

## Qué hay hoy

`soso_web_core::html::resolve_url` devuelve la base **sin tocar** cuando el enlace empieza por
`#`, así que un enlace a un ancla (`<a href="#seccion">`) pierde el ancla. Y un enlace que sólo
trae consulta (`?pagina=2`) acaba pegado al final de la ruta como si fuera un archivo
(`…/pagina.html/?pagina=2`).

## Qué se pide (RFC 3986 §5.2)

- `#nuevo` sobre `https://h/a/p.html?x=1#viejo` da `https://h/a/p.html?x=1#nuevo`
  (se conserva la consulta y **se sustituye** el fragmento);
- `?y=2` sobre `https://h/a/p.html?x=1#viejo` da `https://h/a/p.html?y=2`
  (se sustituyen la consulta **y** el fragmento);
- el enlace **vacío** sigue devolviendo la base tal cual;
- lo demás (URL absolutas, rutas con `/`, relativas) no cambia.

## Alcance

Solo `crates/soso-web-core/src/html.rs`, función `resolve_url`.

## Comprobación

Ejecuta `cargo test -p soso-web-core` antes y después de tu cambio y resume el resultado.
