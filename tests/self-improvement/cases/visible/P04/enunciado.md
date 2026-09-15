# P04 — Acumular sin desbordar un contador

Primera línea de stdin: un entero `tope` (cabe en `u32`). Las líneas
siguientes son sumandos, también `u32`, una por línea; puede no haber ninguna.

Parte de un acumulador a 0, súmale cada sumando y escribe el valor final
seguido de un salto de línea. La suma **satura** en `tope`: nunca lo supera y
nunca envuelve. Ojo con los sumandos cercanos al máximo de `u32`.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
