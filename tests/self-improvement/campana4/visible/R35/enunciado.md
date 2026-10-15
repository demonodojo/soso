# R35 — `gguf2som`: un GGUF malformado no debe tirar el proceso

## Qué hay hoy
`convert` (en `gguf2som`) lee la cabecera de un GGUF —metadatos y descriptores de tensores— antes
de convertir nada. Ese análisis se fía de las longitudes que declara el propio fichero:

- una cadena (clave de metadato, valor, nombre de tensor) se lee reservando de golpe toda la
  longitud declarada. Con una longitud absurda (`u64::MAX`, `1 << 40`…) el proceso entra en pánico o
  aborta por falta de memoria en lugar de devolver un error, que es lo que debe pasar con un
  fichero descargado a medias o corrupto;
- el número de dimensiones de un tensor tampoco se acota: un `n_dims` de miles de millones reserva
  el vector de dimensiones antes de comprobar que el fichero tenga esos datos.

## Qué se pide
Que `convert` devuelva `Err` en esos dos casos, sin pánico ni aborto y sin reservar memoria
proporcional a lo que *dice* el fichero:

- una cadena declarada de más de **16 MiB** (16 · 1024 · 1024 bytes) se rechaza con un error cuyo
  mensaje contiene la palabra `cadena`. Una cadena de hasta 16 MiB, si el fichero la trae entera, se
  sigue leyendo con normalidad. La memoria se reserva a medida que se leen los datos, no por
  adelantado: una cadena cuyos datos faltan (fichero truncado) sigue dando error, y no debe haber
  una reserva de la longitud declarada previa a la lectura;
- un tensor con más de **8** dimensiones se rechaza con un error cuyo mensaje contiene la palabra
  `dimensiones` y el nombre del tensor, **antes** de leer ninguna de sus dimensiones. Hasta 8 se
  acepta como hoy.

El resto del análisis y de la conversión no cambia.

## Alcance
Solo `crates/gguf2som/src/lib.rs` (el análisis de la cabecera GGUF y sus lectores de bajo nivel).

## Comprobación
`cargo test -p gguf2som`, antes y después de tu cambio.
