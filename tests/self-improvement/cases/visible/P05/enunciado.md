# P05 — Normalizar una ruta relativa sin dejarla escapar

Primera línea de stdin: la raíz lógica (por ejemplo `/raiz`). Segunda línea: la
ruta candidata.

Escribe una línea:

- `RECHAZADA` si la ruta es absoluta (empieza por `/`) o si, al resolverla,
  sale por encima de la raíz.
- `OK <ruta>` en otro caso, con la ruta ya normalizada: sin segmentos `.`, con
  los `..` resueltos y sin barras repetidas. Si al normalizar no queda nada,
  la ruta normalizada es `.`.

`..` solo cuenta como subida cuando es un segmento entero: `a..b` es un nombre
corriente y no debe rechazarse.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
