# R14 — `Manifest::find_file` con rutas «sucias»

## Qué hay hoy
`Manifest::find_file("etc/hostname")` compara la ruta tal cual con las del manifiesto, que van
**relativas y sin prefijo**. Una consulta como `./etc/hostname` o `/etc/hostname` (como la escribe
quien lee un `ls` o una ruta absoluta del guest) no encuentra nada.

## Qué se pide
`find_file` debe **quitar de la consulta** los prefijos iniciales `./` y `/`, repetidos si hace
falta (`./etc/x`, `/etc/x`, `.//etc/x`, `/./etc/x` encuentran `etc/x`). No normaliza nada más:
`etc/../etc/x` y `etc//x` siguen sin coincidir, y una ruta que no está sigue dando `None`.
Las rutas del manifiesto no cambian.

## Alcance
Solo `crates/soso-update-core/src/manifest.rs`, función `Manifest::find_file`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.
