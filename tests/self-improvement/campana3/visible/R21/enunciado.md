# R21 — `resolve_redirect`: `//host/ruta` y rutas relativas

## Qué hay hoy
`resolve_redirect(base, loc)` (en `soso-http`) decide a qué URL salta el cliente cuando un servidor
contesta con un redirect y una cabecera `Location`. Hoy sólo entiende dos formas y falla con otras:

- `//cdn.ejemplo.org/x` (referencia de red, la que usan muchos CDN) se trata como una ruta que
  empieza por `/` y sale `https://host-de-la-base//cdn.ejemplo.org/x`, que es basura;
- una ubicación relativa como `x/y.bin` se devuelve tal cual, sin host, y la siguiente petición
  falla al analizar la URL;
- el host de la base se corta sólo por `/`, así que una base como `https://h.org?q=1` da el host
  `h.org?q=1`.

Además la función es privada, y por eso no se puede probar desde fuera.

## Qué se pide
Hacer `resolve_redirect` **pública** (`pub fn resolve_redirect(base: &str, loc: &str) -> String`,
exportada desde la raíz del crate) y que resuelva así (la base siempre es `https://…`):

- `loc` que empieza por `https://` o `http://`: se devuelve sin cambios;
- `loc` que empieza por `//`: `https:` seguido de `loc` (`//otro.org/x` pasa a `https://otro.org/x`);
- `loc` que empieza por `/`: `https://<host de la base><loc>`. El host (con su puerto, si lo hay)
  termina en el primer `/`, `?` o `#`, o al acabar la cadena;
- cualquier otra `loc` es relativa: se coloca tras el **directorio** de la ruta de la base, es decir
  hasta el último `/` de esa ruta inclusive, ignorando la query (`?…`) y el fragmento (`#…`) de la
  base. Si la base no tiene ruta, el directorio es `/`. `c` sobre `https://h.org/a/b` da
  `https://h.org/a/c`; `c` sobre `https://h.org` da `https://h.org/c`.

No se normalizan `.` ni `..` (`../c` sobre `https://h.org/a/b` da `https://h.org/a/../c`).
Si la base no empieza por `https://`, se mantiene el comportamiento actual del host de reserva
(`huggingface.co`). El resto del crate no cambia.

## Alcance
Solo `crates/soso-http/src/lib.rs`, función `resolve_redirect`.

## Comprobación
`cargo test -p soso-http`, antes y después de tu cambio.
