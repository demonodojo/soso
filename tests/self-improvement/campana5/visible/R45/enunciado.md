# R45 — Las redirecciones relativas y las cabeceras en mayúsculas fallan en `soso-http`

## Qué hay hoy
El cliente HTTPS de `soso-http` sigue las redirecciones (3xx) leyendo la cabecera `Location` y
calculando la URL siguiente con la función `resolve_redirect(base, loc)`, que hoy es privada. Dos
síntomas observables:

- Un servidor que responde `Location: next/page` (o `../otra`, `./x`, `?p=2`, `//cdn.h.org/f`) a
  una petición a `https://h.org/a/b/c` acaba en un error de URL: hoy la función devuelve el
  `Location` tal cual, que no es una URL válida. Además, si la URL base lleva query pero no ruta
  (`https://h.org?x=1`) y el `Location` empieza por `/`, el «host» que se usa incluye `?x=1`.
- `header_value(&headers, "Content-Type")` no encuentra nada aunque la cabecera esté en la lista:
  la búsqueda exige que el nombre pedido y el almacenado coincidan letra a letra, de modo que
  sólo funciona si quien llama escribe el nombre en minúsculas y la lista también lo tiene así.

## Qué se pide
1. `resolve_redirect` pasa a ser **pública** (exportada desde la raíz del crate), con la misma firma:
   `pub fn resolve_redirect(base: &str, loc: &str) -> String`. `base` es siempre una URL
   `https://host[:puerto][/ruta][?query][#fragmento]`. Se ignoran los espacios al principio y al
   final de `loc`. Casos, en este orden:
   - `loc` empieza por `https://` o `http://`: se devuelve sin cambios.
   - `loc` empieza por `//`: se devuelve `https:` seguido de `loc`.
   - `loc` empieza por `/`: esquema y autoridad de `base` (todo lo que hay entre `https://` y el
     primer `/`, `?` o `#`, puerto incluido) más `loc`.
   - `loc` empieza por `?`: `base` sin su query ni su fragmento, más `loc`. Si `base` no tiene
     ruta, se usa `/` como ruta (`https://h.org?x=1` + `?p=2` da `https://h.org/?p=2`).
   - cualquier otro `loc` es una ruta relativa: se toma la ruta de `base` (sin query ni fragmento)
     hasta su último `/` inclusive (si `base` no tiene ruta, `/`) y se le añade `loc`.
2. En los resultados de los dos últimos casos con ruta (el de `/` y el de ruta relativa), los
   segmentos `.` y `..` de la **ruta** (lo anterior al primer `?` o `#` de `loc`) se eliminan: `.`
   desaparece, `..` quita el segmento anterior y un `..` que intente subir por encima de la raíz
   se ignora. Si el último segmento es `.` o `..`, la ruta resultante termina en `/`. La query y el
   fragmento de `loc` se dejan intactos (no se normalizan aunque contengan `..`).
3. `header_value` ignora mayúsculas y minúsculas tanto en el nombre pedido como en los nombres de
   la lista, y devuelve el valor de la **primera** cabecera de la lista que coincida.

Ejemplos con base `https://h.org:8443/a/b/c?z=1#f`:
`next/page` → `https://h.org:8443/a/b/next/page`; `../d` → `https://h.org:8443/a/d`;
`../../../d` → `https://h.org:8443/d`; `.` → `https://h.org:8443/a/b/`; `?p=2` →
`https://h.org:8443/a/b/c?p=2`; `/x/./y?q=a/../b` → `https://h.org:8443/x/y?q=a/../b`.

## Alcance
Sólo `crates/soso-http/src/lib.rs`: `resolve_redirect` y `header_value`. No cambies las demás firmas
públicas; el resto del crate (que ya llama a `resolve_redirect`) sigue funcionando igual.

## Comprobación
`cargo test -q -p soso-http --features std`, antes y después de tu cambio.
