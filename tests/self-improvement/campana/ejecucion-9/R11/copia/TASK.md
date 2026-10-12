# R11

# R11 — `semver::parse`: prefijo `v` y componentes de más

## Qué hay hoy
`soso_update_core::semver::parse` convierte `major.minor.patch`, pero las etiquetas de versión
suelen llevar un prefijo `v` (`v1.2.3`) y hoy ese texto da `None`; y una versión con un cuarto
componente (`1.2.3.4`) se acepta ignorándolo en silencio, como si fuera `1.2.3`.

## Qué se pide
- `v1.2.3` y `V1.2.3` se parsean igual que `1.2.3`;
- `1.2.3.4` (más de tres componentes) da `None`;
- lo que ya devolvía `None` (`1.2`, `1..3`, `a.b.c`, vacío) sigue devolviendo `None`;
- los sufijos tras `-` o `+` se siguen ignorando (`1.2.3-rc1`, `v1.2.3+build5`).

## Alcance
Solo `crates/soso-update-core/src/semver.rs`, función `parse`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.


## Qué puedes tocar

- `crates/soso-update-core/src/semver.rs`

Cualquier otra ruta queda fuera: un cambio ahí no se aplica.

## Qué tiene que pasar

- `cargo test -q -p soso-update-core --features std` (en `.`, máximo 900 s)

## Límites

- Intentos: 1
- Herramientas: 30
- Segundos: 3600
