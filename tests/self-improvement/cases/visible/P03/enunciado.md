# P03 — Leer Content-Length de un bloque de cabeceras

stdin es un bloque de cabeceras HTTP, una por línea, con el formato
`Nombre: valor`. No hay línea de petición ni cuerpo.

Escribe **una** línea:

- `OK <n>` si hay exactamente un valor de `Content-Length` utilizable. El
  nombre se compara **sin distinguir mayúsculas** (RFC 9110 §5.1) y el valor se
  recorta por los lados.
- `ERROR duplicado` si aparece más de una vez con valores distintos. Si
  aparece repetida con el mismo valor, es `OK <n>`.
- `ERROR invalido` si el valor no es una secuencia no vacía de dígitos ASCII
  (nada de signos, espacios interiores ni listas separadas por comas).
- `ERROR ausente` si no aparece.

Si concurren varios motivos, gana el primero de esta lista: duplicado,
invalido, ausente.

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
