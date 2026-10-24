# R44 — `semver::comparar` y `semver::elegir_mas_nueva`: pre-releases y metadatos

## Qué hay hoy
`soso-update-core/src/semver.rs` sólo sabe `major.minor.patch`: `parse` descarta en silencio todo lo que
va tras un `-` o un `+`, de modo que `0.4.0-rc.1` y `0.4.0` son «la misma versión». Con canales de
prueba (`dev`, candidatas a release) hace falta saber que `0.4.0-rc.1` es **anterior** a `0.4.0`, y
poder elegir la más nueva de una lista de versiones publicadas sin tragarse una cadena inválida.

## Qué se pide
Añadir a `crates/soso-update-core/src/semver.rs`, sin tocar `parse`, `cmp` ni `SemVer`, dos funciones
públicas nuevas:

```rust
pub fn comparar(a: &str, b: &str) -> Option<core::cmp::Ordering>
pub fn elegir_mas_nueva<'a>(candidatas: &[&'a str], con_prerelease: bool) -> Option<&'a str>
```

**Formato válido** (SemVer 2.0.0): `MAJOR.MINOR.PATCH`, seguido opcionalmente de `-PRERELEASE` y de
`+BUILD`. Se recortan los espacios de los extremos de la cadena entera antes de analizarla.

- `MAJOR`, `MINOR` y `PATCH` son exactamente tres números decimales (`u32`), sin signo, sin componentes
  vacíos, sin ceros a la izquierda (`0` vale, `01` no) y sin un cuarto componente. Un número que no
  quepa en `u32` es inválido.
- `PRERELEASE` (tras el primer `-`) son identificadores separados por `.`; cada uno es no vacío y sólo
  usa `[0-9A-Za-z-]`; un identificador formado sólo por dígitos no puede tener ceros a la izquierda
  (`1.0.0-01` es inválido, `1.0.0-0` y `1.0.0-0a` valen). Un guion dentro del pre-release es parte de un
  identificador (`1.0.0-alpha-beta` tiene un solo identificador).
- `BUILD` (tras el primer `+`) son identificadores separados por `.`, no vacíos, de `[0-9A-Za-z-]`;
  aquí los ceros a la izquierda sí se permiten. `1.0.0+` es inválido.

**`comparar`** devuelve `None` si alguna de las dos cadenas no es válida; si ambas lo son, su precedencia:

1. se comparan `MAJOR`, `MINOR`, `PATCH` en ese orden, numéricamente (`0.10.0 > 0.2.0`);
2. con el mismo núcleo, una versión con pre-release es **menor** que la misma sin él;
3. con pre-release en las dos, se comparan los identificadores de izquierda a derecha: dos numéricos
   se comparan como números (sin límite de tamaño: `1.0.0-9` < `1.0.0-10`, también con 30 dígitos);
   un identificador numérico es menor que uno alfanumérico; dos alfanuméricos se comparan por orden
   ASCII; si todos los comparados son iguales, el pre-release con menos identificadores es el menor;
4. los metadatos de build **no cuentan**: `1.0.0+a` y `1.0.0+b` son `Some(Equal)`.

**`elegir_mas_nueva`** devuelve la cadena de `candidatas` de mayor precedencia, tal cual venga escrita
(sin recortar). Ignora las cadenas inválidas; si `con_prerelease` es `false`, ignora también las que
llevan pre-release. Si dos candidatas válidas empatan en precedencia (por ejemplo, sólo difieren en el
build), gana la que aparece **antes** en la lista. Devuelve `None` si no queda ninguna.

## Alcance
Sólo `crates/soso-update-core/src/semver.rs`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio; las pruebas existentes deben seguir
pasando.
