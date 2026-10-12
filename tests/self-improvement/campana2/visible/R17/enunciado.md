# R17 — `resolve_url`: `//host/ruta` y base `http://`

## Qué hay hoy
`resolve_url(base, href)`:
- un enlace **relativo al protocolo** (`//cdn.ejemplo.org/x.css`) se trata como una ruta que empieza
  por `/` y sale `https://host//cdn.ejemplo.org/x.css` (basura);
- un enlace que empieza por `/` sólo se resuelve si la base es `https://`; con una base
  `http://…` devuelve el href tal cual, sin host.

## Qué se pide
- `//otro.org/x` toma el esquema de la base: `https://otro.org/x` con base `https://…` y
  `http://otro.org/x` con base `http://…`;
- `/x` sobre `http://h/a/b` da `http://h/x` (igual que ya hace con `https`);
- lo demás no cambia: URL absolutas, rutas relativas, enlaces vacíos.

## Alcance
Solo `crates/soso-web-core/src/html.rs`, función `resolve_url`.

## Comprobación
`cargo test -p soso-web-core`, antes y después de tu cambio.
