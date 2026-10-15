# R37 — `parse_url`: query sin ruta, fragmento, host vacío y puerto inválido

## Qué hay hoy
`parse_url(url)` (en `soso-http`) separa una URL `https://…` en `(esquema, host, puerto, ruta)`; la usan
todas las peticiones del cliente, incluidas las redirecciones. Hoy es privada y acepta cosas que luego
rompen la petición:

- el host sólo termina en el primer `/`: en `https://h.org?q=1` el host sale `h.org?q=1`;
- el fragmento (`#…`) viaja dentro de la ruta, aunque no debe llegar al servidor;
- un host vacío (`https:///x`, `https://:443/x`) o un puerto vacío o `0` (`https://h.org:/x`) se aceptan;
- las credenciales (`https://usuario:clave@h.org/x`) acaban convertidas en host y puerto absurdos.

## Qué se pide
Hacer `parse_url` **pública** (`pub fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError>`,
exportada desde la raíz del crate), con la misma forma de resultado, y que cumpla:

- sólo se admite el esquema `https://`; cualquier otra cosa da `Err(HttpError::Parse)`;
- el fragmento (desde el primer `#`) se descarta antes de analizar nada más;
- el host (con su puerto opcional) termina en el primer `/` o `?`, o al acabar la cadena. La ruta
  devuelta es lo que sigue, query incluida; si no hay nada es `/`, y si lo que sigue empieza por `?`
  la ruta es `/` seguida de eso (`https://h.org?q=1` da ruta `/?q=1`);
- un host vacío, un puerto vacío, `0` o que no quepa en `u16`, y la presencia de `@` en la parte del
  host, dan `Err(HttpError::Parse)`;
- sin puerto explícito, el puerto es 443.

## Alcance
Solo `crates/soso-http/src/lib.rs`, función `parse_url`. Los llamadores actuales no cambian.

## Comprobación
`cargo test -p soso-http`, antes y después de tu cambio.
