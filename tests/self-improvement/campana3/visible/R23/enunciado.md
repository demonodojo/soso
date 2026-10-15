# R23 — `reflow_html` con listas sin cerrar y reglas `<hr>`

## Qué hay hoy
`reflow_html` convierte HTML en texto plano. Dos formas de HTML muy comunes salen mal:

- Un `<li>` **sin** `</li>` (válido en HTML: `<ul><li>uno<li>dos<li>tres</ul>`) no empieza línea
  nueva, de modo que los elementos quedan pegados: `• uno • dos • tres` en una sola línea.
- Una regla horizontal escrita como `<hr>` (la forma normal) no dibuja nada; solo la forma
  `<hr/>` produce la línea de guiones.

## Qué se pide
1. Cada `<li>` de apertura debe **empezar en una línea nueva**, esté o no cerrado el anterior.
   Cada elemento sale como una línea con la viñeta que ya se pone hoy, un espacio y su texto
   (sin línea en blanco entre elementos consecutivos). La viñeta no se toca.
2. `<hr>` y `<hr/>` (y `<HR>`) deben dibujar la misma regla: una línea propia de guiones `-`
   de longitud `min(ancho, 40)`, donde `ancho` es el que ya se usa hoy (el pedido, con mínimo 20).
   El texto anterior y el posterior a la regla quedan en líneas distintas de la regla.

Lo demás no cambia: bloques, enlaces numerados, `<br>`, `<script>`/`<style>` omitidos, etc.

## Alcance
Solo `crates/soso-web-core/src/html.rs`, función `reflow_html`.

## Comprobación
`cargo test -p soso-web-core`, antes y después de tu cambio.
