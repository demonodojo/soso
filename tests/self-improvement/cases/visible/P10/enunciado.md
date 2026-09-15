# P10 — Liberar el recurso también en el camino de error

stdin trae una única palabra: `ok` o `fallo`.

El programa abre un recurso al empezar. Al terminar, el recurso debe liberarse
**exactamente una vez**, y su liberación escribe la línea `liberado`.

- Con `ok`: escribe `hecho`, luego `liberado`, y termina con código 0.
- Con `fallo`: no escribe `hecho`; escribe `liberado` y termina con código 1.

La liberación no se escribe a mano en cada salida: se apoya en el momento en
que el recurso deja de existir.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
