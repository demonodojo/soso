# P09 — Pertenencia a un rango semiabierto

Primera línea de stdin: dos enteros `a` y `b` separados por un espacio, que
definen el rango **semiabierto** `[a, b)`: incluye `a` y excluye `b`.

Cada línea siguiente es un entero. Por cada una escribe `SI` si pertenece al
rango y `NO` si no. Si `b <= a` el rango está vacío y nada pertenece.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
