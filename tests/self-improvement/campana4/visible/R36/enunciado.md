# R36 — `Manifest::parse`: versiones desconocidas y `num_layers` sin respaldo

## Qué hay hoy
`Manifest::parse` (en `sosomodel`) lee el `manifest.som` de un modelo. Hay dos entradas que acepta
cuando no debería:

- la versión de la cabecera no se comprueba: un fichero con versión 0, o con una versión posterior
  a las que conoce el crate (hoy la 6 es la última), se interpreta a ciegas como si fuera la más
  cercana, y un manifest de un formato futuro se lee con campos desalineados en vez de rechazarse;
- en los manifests anteriores a la v4 no hay tabla de capas que respalde `num_layers`, así que el
  parser sintetiza una capa por cada una que el fichero afirma tener. Una cabecera corrupta con
  `num_layers = 4_000_000_000` intenta construir cuatro mil millones de capas.

## Qué se pide
Que `Manifest::parse` devuelva `Err(())` cuando:

- la versión de la cabecera sea 0 o mayor que 6;
- la versión sea 1, 2 o 3 y `num_layers` supere **1024**.

Las versiones 4 a 6 no tienen ese techo de capas (su tabla de `LayerSpec` ya debe coincidir con
`num_layers`, como hoy), y un manifest válido de cualquier versión 1–6 se sigue leyendo igual que
ahora. La serialización no cambia.

## Alcance
Solo `crates/sosomodel/src/manifest.rs`, función `Manifest::parse` (y las constantes que necesites
junto a ella).

## Comprobación
`cargo test -p sosomodel`, antes y después de tu cambio.
