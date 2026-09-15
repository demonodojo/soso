# R02 — Validar el CRC32 de la cabecera GPT al parsearla

## Qué hay hoy

`gptdisk::Header::parse` comprueba la firma `EFI PART` y que los campos sean
plausibles, pero **no** verifica el CRC32 de la cabecera, aunque
`Header::render` sí lo calcula y `crc32_ieee` ya existe en el crate.

Una tabla con un byte cambiado se lee como buena y el disco se reparticiona a
partir de datos que nadie ha validado.

## Qué se pide

Que `parse` valide el CRC32 de la cabecera según UEFI 2.10 §5.3:

- el CRC cubre los primeros `header_size` bytes,
- con el propio campo de CRC (offset 16..20) puesto a cero para el cálculo,
- si no coincide, `parse` devuelve `Err` con una variante propia del error
  (`BadCrc` es un buen nombre) y su texto en `Display`.

Una cabecera recién generada por `render` debe seguir parseándose sin cambios,
y el resto de la suite del crate debe seguir pasando.

## Alcance

Solo `crates/gptdisk/src/lib.rs`.
