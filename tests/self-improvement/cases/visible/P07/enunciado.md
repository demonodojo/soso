# P07 — Un iterador agotado se queda agotado

stdin trae un único entero `n`.

Implementa un iterador propio que produzca `0, 1, ..., n-1` y después nada
más, para siempre. Llama a `next()` exactamente `n + 3` veces y escribe los
resultados separados por un espacio, en una sola línea terminada en salto de
línea: el valor si lo hay, o `-` si el iterador ya no produce nada.

Con `n = 0` la línea tiene tres `-`.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
