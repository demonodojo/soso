# P02 — Escapar una cadena para JSON

La entrada es todo stdin quitando un único salto de línea final si lo hay.

Escribe en stdout esa entrada como **cadena JSON**, con sus comillas dobles
incluidas, y un salto de línea al final. Reglas de escape (RFC 8259 §7):

- `"` se escapa como `\"`; la barra invertida, como `\\`
- retroceso `\b`, avance de página `\f`, salto `\n`, retorno `\r`, tabulador `\t`
- cualquier otro carácter de control por debajo de `0x20` pasa a `\u00xx`,
  con los dígitos hexadecimales **en minúscula**
- todo lo demás, incluido el texto no ASCII, va literal en UTF-8

## Contrato del candidato

- Un único archivo Rust que compile con `rustc --edition 2021 -O`, solo `std`.
- Lee **todo** stdin y escribe el resultado en stdout.
- Termina con código 0 salvo que el enunciado diga otra cosa.
- Nada de dependencias, hilos ni red.
