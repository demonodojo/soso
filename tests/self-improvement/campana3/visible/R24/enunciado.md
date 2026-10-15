# R24 — `gptdisk::Header::parse` acepta cabeceras incoherentes

## Qué hay hoy
`Header::parse` solo comprueba la firma, que `header_size` esté entre 92 y 512, que
`num_entries` esté entre 1 y 512 y que `entry_size` sea al menos 128 y múltiplo de 8. Con
eso deja pasar cabeceras que la especificación UEFI no permite y que luego descuadran
`relayout` y el listado de particiones.

## Qué se pide
`Header::parse` debe devolver `Err(Error::BadHeader)` además cuando:

- `entry_size` no es de la forma `128 * 2^n` (es decir 128, 256, 512, 1024…). Por ejemplo
  136, 192, 384 y 640 se rechazan; 128, 256 y 512 se aceptan.
- el rango utilizable es incoherente: `first_usable > last_usable`. Un rango de un solo
  sector (`first_usable == last_usable`) sigue siendo válido.

La firma incorrecta sigue dando `Error::BadSignature` y se comprueba antes que lo demás.
Una cabecera correcta (la que produce `Header::render`) se sigue leyendo igual que hoy.

## Alcance
Solo `crates/gptdisk/src/lib.rs`, función `Header::parse`.

## Comprobación
`cargo test -p gptdisk`, antes y después de tu cambio.
