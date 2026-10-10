# R03 — header_value sin distinguir mayúsculas

## Qué hay hoy

`soso_http::header_value` (función `header_value` de `crates/soso-http/src/lib.rs`) compara
el nombre de la cabecera con `==`, es decir, distinguiendo mayúsculas. El parser de
respuestas guarda los nombres en minúscula, así que una consulta escrita como en la
especificación (`Content-Length`) no encuentra nada y devuelve `None`.

## Qué se pide

Que la comparación del nombre sea **insensible a mayúsculas** (RFC 9110 §5.1), sin cambiar
la firma pública ni el resto del comportamiento:

- sigue devolviendo la **primera** aparición;
- sigue devolviendo `None` cuando la cabecera no está;
- los valores se devuelven tal cual están guardados.

## Alcance

Solo `crates/soso-http/src/lib.rs`. El archivo es largo (más de mil líneas): localiza la
función con `grep` y lee sólo ese tramo con `read`. En las llamadas, `offset` y `limit` son
números enteros literales (por ejemplo `"offset": 800, "limit": 60`), nunca expresiones.

## Comprobación

Ejecuta `cargo test -p soso-http` antes y después de tu cambio. Cuando termines, resume qué
cambiaste y el resultado de la prueba.
