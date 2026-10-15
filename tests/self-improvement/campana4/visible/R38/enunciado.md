# R38 — `maybe_decompress`: gzip real y deflate sin cabecera zlib

## Qué hay hoy
`maybe_decompress(body, content_encoding)` (en `soso-web-core`, módulo `decode`) descomprime el cuerpo de
una respuesta HTTP según `Content-Encoding`. Hoy trata `gzip` y `deflate` igual, como un flujo zlib, y
por eso:

- un cuerpo `gzip` de verdad (cabecera `1f 8b`, deflate crudo y cola de 8 bytes) siempre da error;
- un cuerpo `deflate` que el servidor mandó sin la cabecera zlib (cosa frecuente) también da error.

## Qué se pide
Sin cambiar la firma de la función ni su comportamiento para «sin codificación», `identity` y
codificaciones desconocidas (que devuelven el cuerpo tal cual):

- **gzip** (cualquier valor que contenga `gzip`, sin distinguir mayúsculas, como `x-gzip`): analizar el
  miembro gzip según el RFC 1952. Se exige cabecera de al menos 10 bytes con magia `1f 8b` y método 8;
  según los flags se saltan el campo extra (FEXTRA, con su longitud de 2 bytes little-endian), el nombre
  (FNAME) y el comentario (FCOMMENT), ambos terminados en un byte 0, y los 2 bytes de CRC de cabecera
  (FHCRC). Tras la cabecera vienen los datos (deflate crudo) y una cola de 8 bytes: CRC32 (no hace falta
  comprobarlo) y el tamaño original módulo 2^32, que sí debe coincidir con la longitud del resultado.
- Cualquier cuerpo gzip malformado o truncado (magia o método erróneos, campos que se salen del cuerpo,
  sin cola, datos corruptos, tamaño que no cuadra) devuelve `Err(())`, sin entrar en pánico.
- **deflate** (valores que contengan `deflate`): se intenta primero como zlib y, si falla, como deflate
  crudo. Si ambos fallan, `Err(())`.

## Alcance
Solo `crates/soso-web-core/src/decode.rs` (`miniz_oxide` ya es dependencia; puedes añadir funciones
auxiliares privadas).

## Comprobación
`cargo test -p soso-web-core`, antes y después de tu cambio.
