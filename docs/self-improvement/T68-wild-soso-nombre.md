# T68 — El enlazador `wild-soso` no existe con ese nombre

**Origen:** medido en [T38](T38-toolchain-inventario.md), fuera de su alcance.
**Aplica en:** `tools/wild-soso/`, `targets/x86_64-unknown-soso.json`,
`scripts/soso-rust-bootstrap.sh`, `xtask/src/main.rs` · **Estado:** **hecha** (2026-09-25).

## Lo que no cuadra

`tools/wild-soso/Cargo.toml` declara:

    [[bin]]
    name = "wild"

Y quien lo busca lo busca por otro nombre:

- `targets/x86_64-unknown-soso.json` → `"linker": "wild-soso"`
- el `config.toml` que genera el bootstrap →
  `linker = "$ROOT/tools/wild-soso/target/release/wild-soso"`

Ese fichero **no existe nunca**: `cargo build -p wild-soso` produce
`target/debug/wild` (y `target/release/wild`), no `wild-soso`. Comprobado:
`cargo build -p wild-soso` termina con éxito y `target/debug/wild-soso` no
está.

## El detalle que lo empeora

El envoltorio invoca `Command::new("wild")`. Las instrucciones del bootstrap
dicen poner `tools/wild-soso/target/release` en el `PATH`. Si alguien las
sigue, el binario que hay en ese directorio se llama `wild`, así que el
envoltorio **se llamaría a sí mismo**.

## Y además `wild` no está instalado

    $ command -v wild
    (nada)

Así que el target `x86_64-unknown-soso` no puede enlazar hoy ni arreglando los
nombres. El envoltorio ya lo dice bien cuando llega a ejecutarse —sale con 127
y explica `cargo install wild-linker`—; el problema es que con los nombres
actuales no llega.

## Y dos desajustes más, del mismo tipo

Al arreglar el nombre aparecieron dos rutas que tampoco existen nunca:

1. El bootstrap ponía en el `PATH` —y usaba como `linker =`—
   `tools/sosoas/target/release` y `tools/wild-soso/target/release`. Los dos
   crates son **miembros del workspace**, así que sus `target/` por crate no se
   crean jamás: todo va a `target/` de la raíz. `xtask rust-build-std` tenía
   las mismas dos rutas.
2. El bootstrap compilaba **sin `--release`** mientras todas esas rutas decían
   `release`.

Es la misma clase de fallo que el nombre: rutas que nombran cosas que no
existen, en un camino que nadie recorre todavía.

## Hecho

- `[[bin]] name = "wild-soso"`, que es lo que pide el target.
- El bootstrap y `xtask` apuntan a `target/release` del workspace, y el
  bootstrap compila con `--release`.
- Con el nombre corregido **desaparece la recursión**: el envoltorio llama a
  `wild` y en ese directorio hay un `wild-soso`, así que encuentra el real o
  falla con su mensaje de siempre.

## Comprobación

Un test en `tools/wild-soso` lee `targets/x86_64-unknown-soso.json` y exige que
su `"linker"` sea igual a `CARGO_BIN_NAME`. Los dos ficheros ya se habían
separado una vez sin que nadie lo notara; ahora no pueden.

Comprobado que **puede fallar**: devolviendo el binario a `wild`, el test cae
con `left: "wild-soso" / right: "wild"`.

    cargo test -p wild-soso   1/1

## Lo que sigue faltando

`wild` **no está instalado** en este host, así que el target sigue sin poder
enlazar. Eso no lo arregla esta ficha —es instalar una herramienta, o portarla,
y eso es T39–T42—; lo que cambia es que ahora el fallo llega hasta el mensaje
que lo explica en vez de perderse en un nombre que no existe.
