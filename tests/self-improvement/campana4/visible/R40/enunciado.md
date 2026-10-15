# R40 — `soso-abi`: validar los flags de `open`

## Qué hay hoy
`soso-abi` define los flags de `open` (`O_RDONLY`, `O_WRONLY`, `O_APPEND`, `O_CREAT`, `O_TRUNC`,
`O_EXCL`) y documenta cómo se combinan: `O_APPEND`, `O_CREAT` y `O_TRUNC` sólo tienen sentido
«con `O_WRONLY`», y `O_EXCL` sólo «con `O_CREAT`». Pero esas reglas están únicamente en los
comentarios: nada las comprueba, y cada llamante (kernel, libsoso, herramientas de host) tendría
que reimplementarlas. Hoy una llamada como `open(path, O_EXCL)` o `open(path, 1 << 40)` se
interpreta de forma silenciosamente distinta según quién la reciba.

## Qué se pide
Añadir a la raíz del crate dos elementos públicos:

- `pub const O_FLAGS_MASK: u64`: la unión de todos los bits de flag que el kernel conoce.
- `pub fn open_flags_validos(flags: u64) -> Result<(), i64>`: devuelve `Ok(())` si la combinación
  es coherente y `Err(-EINVAL)` (el errno en negativo, como el resto de la ABI) si:
  - hay algún bit fuera de `O_FLAGS_MASK`;
  - se usa `O_APPEND`, `O_CREAT`, `O_TRUNC` u `O_EXCL` sin `O_WRONLY`;
  - se usa `O_EXCL` sin `O_CREAT`;
  - se piden a la vez `O_APPEND` y `O_TRUNC`, que se contradicen (conservar y vaciar).

`O_RDONLY` solo (valor 0) es válido. Los valores y el resto de la API del crate no cambian.

## Alcance
Solo `crates/soso-abi/src/lib.rs`. No hace falta tocar el kernel ni los usuarios.

## Comprobación
`cargo test -p soso-abi`, antes y después de tu cambio.
