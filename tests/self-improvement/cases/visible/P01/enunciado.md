# P01 — Truncar a N bytes sin partir un carácter

Primera línea de stdin: un entero `N`, el límite **en bytes**. El resto de
stdin, quitando un único salto de línea final si lo hay, es el texto.

Escribe el prefijo más largo del texto cuya longitud en bytes sea `<= N`,
seguido de un salto de línea. Nunca partas un carácter UTF-8: si el siguiente
carácter no cabe entero, el prefijo termina antes.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
