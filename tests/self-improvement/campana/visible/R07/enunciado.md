# R07 — `Guid::parse` con llaves

## Qué hay hoy

`gptdisk::Guid::parse` sólo entiende la forma canónica `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
Muchas herramientas (y el registro de Windows) escriben el GUID entre llaves:
`{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`.

## Qué se pide

Que `Guid::parse` acepte **también** la forma entre llaves y dé el mismo `Guid` que la forma
sin ellas:

- `{C12A7328-F81F-11D2-BA4B-00A0C93EC93B}` da lo mismo que `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`;
- una llave sin su pareja (`{…` o `…}`) se rechaza con `None`;
- todo lo que se rechazaba antes se sigue rechazando; la forma sin llaves no cambia.

## Alcance

Solo `crates/gptdisk/src/lib.rs`, función `Guid::parse`.

## Comprobación

Ejecuta `cargo test -p gptdisk` antes y después de tu cambio y resume el resultado.
