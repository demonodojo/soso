# P08 — Ordenar por clave conservando el orden de llegada

Cada línea de stdin es `clave valor`: la clave es un entero y el valor, el
resto de la línea tras el primer espacio.

Escribe las líneas ordenadas por clave ascendente. Entre líneas con la misma
clave se conserva **el orden en que llegaron**. El formato de salida es el
mismo de la entrada.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
