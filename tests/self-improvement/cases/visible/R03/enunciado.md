# R03 — header_value sin distinguir mayúsculas

## Qué hay hoy

`soso_http::header_value` compara el nombre de la cabecera con `==`, es decir,
distinguiendo mayúsculas. El parser de respuestas guarda los nombres en
minúscula, así que una consulta escrita como en la especificación
(`Content-Length`) no encuentra nada y devuelve `None`.

## Qué se pide

Que la comparación del nombre sea **insensible a mayúsculas** (RFC 9110 §5.1),
sin cambiar la firma pública ni el resto del comportamiento:

- sigue devolviendo la **primera** aparición;
- sigue devolviendo `None` cuando la cabecera no está;
- los valores se devuelven tal cual están guardados.

## Alcance

Solo `crates/soso-http/src/lib.rs`.
