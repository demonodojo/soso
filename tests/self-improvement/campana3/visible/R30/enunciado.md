# R30 — Códigos de salida por señal en `soso-abi`

## Qué hay hoy
`soso_abi::exit_by_signal(sig)` devuelve `128 + sig` como `u8`. Con `sig > 127` la suma desborda:
en debug entra en pánico y en release da la vuelta y devuelve un código sin sentido. Además no hay
forma de hacer el camino inverso: quien recoge un código con `wait_decode` (por ejemplo `sosh` al
imprimir «terminado por señal N») tiene que repetir a mano la resta.

## Qué se pide
1. `exit_by_signal(sig)` deja de desbordar: **satura en 255**. Para `sig` de 0 a 127 el resultado no
   cambia (`128 + sig`); para `sig` de 128 a 255 devuelve 255.
2. Nueva función pública `signal_de_salida(code: u8) -> Option<u8>`, la inversa:
   - `code` de 129 a 255 → `Some(code - 128)`.
   - `code` de 0 a 128 → `None` (128 no corresponde a ninguna señal real; el 0 de `SIGPROBE` no
     mata a nadie, y menos de 128 es una salida normal).
   - Para todo `sig` de 1 a 127, `signal_de_salida(exit_by_signal(sig)) == Some(sig)`.
3. `exit_by_signal` sigue siendo `const fn`.

## Alcance
Solo `crates/soso-abi/src/lib.rs`.

## Comprobación
`cargo test -p soso-abi`, antes y después de tu cambio.
