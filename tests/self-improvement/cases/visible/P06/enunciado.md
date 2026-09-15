# P06 — Distinguir «no existe» de otros errores de E/S

stdin trae una ruta por línea. El programa se ejecuta en un directorio de
trabajo donde existen los archivos que el caso prepare.

Por cada ruta escribe una línea:

- `LEIDO <n>` si es un archivo regular que se puede leer, con `n` = bytes leídos.
- `NOEXISTE` si no existe.
- `ERROR` si existe pero la lectura falla por cualquier otro motivo (por
  ejemplo, es un directorio).

No confundas los dos últimos: un error cualquiera no es «no existe».

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
