# R33 — `parse_pcm16_mono`: WAV estéreo, `data` cortado y `data` antes de `fmt`

## Qué hay hoy
`parse_pcm16_mono(data)` (módulo `wav` de `soso-audio`) lee un WAV PCM16 y devuelve
`(frecuencia, muestras)`. Hoy sólo sirve para ficheros "de manual" y rechaza o pierde audio real:

- un WAV **estéreo** de 16 bits da `Unsupported`, aunque cualquier grabadora los produce;
- si el chunk `data` declara más bytes de los que hay (fichero cortado a medias, o cabecera de
  grabación en streaming con tamaño máximo/incorrecto), el chunk se descarta entero y el resultado
  es `BadFmt`, cuando casi todo el audio está ahí;
- si el chunk `data` aparece **antes** que `fmt `, se evalúa sin conocer aún el formato y se
  rechaza como `Unsupported`.

## Qué se pide
Que `parse_pcm16_mono` se comporte así (lo ya correcto no cambia):

- Admite 1 o 2 canales con 16 bits. Con 2 canales, cada muestra de salida es la media entera de
  los dos canales de esa trama: `(izq as i32 + der as i32) / 2` (división entera de Rust),
  convertida a `i16`. Cualquier otro número de canales u otra profundidad de bits es
  `Unsupported`; un `audio_format` distinto de 1 sigue siendo `Unsupported`.
- Un chunk `data` cuyo tamaño declarado excede lo que queda de fichero se lee **hasta el final del
  fichero**. Un último byte suelto (o una trama estéreo incompleta) se ignora.
- El orden de los chunks no importa: `data` puede ir antes o después de `fmt `. Si hay varios
  `data` se concatenan en orden de aparición.
- Sin chunk `fmt ` el resultado es `BadFmt`; sin muestras tras decodificar, también `BadFmt`.
- La cabecera mínima, `RIFF`/`WAVE` y los errores `TooShort` y `BadRiff` no cambian.

## Alcance
Solo `crates/soso-audio/src/wav.rs`, función `parse_pcm16_mono`.

## Comprobación
`cargo test -p soso-audio --features std`, antes y después de tu cambio.
