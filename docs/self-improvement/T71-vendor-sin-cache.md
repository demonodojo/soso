# T71 — `xtask` buscaba el vendor de Rust en un sitio donde nunca está

**Origen:** destapado el 2026-09-25 al correr por primera vez el comando de
comprobación de [T39](T39-bootstrap-libstd.md), con `wild` ya instalado (D3).
**Aplica en:** `xtask/src/main.rs` · **Estado:** **hecha** (2026-09-25).

## Lo que pasaba

    $ cargo xtask rust-build-std
    apply-patches: OK → /home/jmdiez/.cache/soso-rust-vendor
    …
    thread 'main' panicked at xtask/src/main.rs:352:29:
    x.py: No such file or directory (os error 2)

Y `x.py` **estaba**:

    $ ls ~/.cache/soso-rust-vendor/x.py
    -rwxrwxr-x 2,0k x.py
    $ cd ~/.cache/soso-rust-vendor && ./x.py --help
    Usage: x.py <subcommand> …          # rc=0

El mensaje señalaba a un fichero que existe y se ejecuta a mano. Ahí es donde
se pierde media hora.

## Por qué

Las dos mitades del comando no ponían el vendor en el mismo sitio.

El script, bien:

    VENDOR="${SOSO_RUST_VENDOR:-${XDG_CACHE_HOME:-$HOME/.cache}/soso-rust-vendor}"

`xtask`, mal — al caer a `HOME` se dejaba el `.cache`:

```rust
std::env::var("XDG_CACHE_HOME")
    .map(PathBuf::from)
    .unwrap_or_else(|_| std::env::var("HOME").map(PathBuf::from)…)
    .join("soso-rust-vendor")           // $HOME/soso-rust-vendor
```

`XDG_CACHE_HOME` **no está definida** en una sesión normal de este host, así
que `xtask` hacía `current_dir("/home/jmdiez/soso-rust-vendor")`, que no
existe. `Command` da `ENOENT` por el directorio, pero lo cuenta como si fuera
el programa: **«x.py: No such file or directory»**.

Consecuencia: `cargo xtask rust-build-std` —el comando que la propia T39 pone
en su sección de Comprobación— **no ha funcionado nunca** en una máquina sin
`XDG_CACHE_HOME`. El script de bootstrap sí, porque es el que tiene la ruta
buena; por eso el log dice `apply-patches: OK` con la ruta **correcta** dos
líneas antes de fallar con la incorrecta.

## Lo que me costó mirar de más

La primera hipótesis fue que `Command::new("./x.py").current_dir(…)` no
resuelve un camino relativo contra el directorio nuevo — es un aviso que está
en la documentación de `std`. Se comprobó con un programa de doce líneas y
**es falsa aquí**: relativo y absoluto dan los dos rc=0. Sin esa comprobación
habría «arreglado» el camino relativo y el fallo habría seguido igual, con una
explicación plausible encima.

## Hecho

- Una sola función, `vendor_rust()`, con la misma regla que el script, y una
  versión pura `vendor_rust_de(soso, xdg, home)` para poder probarla sin mutar
  el entorno del proceso.
- Si no hay `x.py` en el vendor, el comando **dice eso**: qué directorio miró,
  quién lo prepara y que `SOSO_RUST_VENDOR` elige otro. Un `ENOENT` de
  `current_dir` ya no se puede leer como si faltara el programa.

## Comprobación

Un test le pregunta **al script** dónde cae el vendor —ejecuta su propia línea
`VENDOR=` en un `bash` con el entorno controlado— y exige que `xtask` diga lo
mismo, con y sin `XDG_CACHE_HOME`. Es la misma forma que el test de
[T68](T68-wild-soso-nombre.md): dos ficheros que tienen que coincidir se atan,
en vez de confiar en que nadie los separe.

Comprobado que **puede fallar**: devolviendo el `.join(".cache")`, el test cae
con `left: "/tmp/casa/soso-rust-vendor" / right: "/tmp/casa/.cache/soso-rust-vendor"`.

    cargo test -p xtask --bin xtask vendor_rust   2/2

## De la misma familia

Es el tercer caso seguido de **una ruta que nombra algo que no existe**, en el
mismo camino de SI-7 y encontrado siempre igual, ejecutando en vez de leer:
[T67](T67-sosoas-elf-desplazado.md) (offsets del ELF), [T68](T68-wild-soso-nombre.md)
(el binario `wild` contra el `linker` `wild-soso`, y dos `target/` por crate que
en un workspace no se crean jamás) y esta. Ninguno de los tres se ve revisando
el código: los tres se ven al correrlo.

## Reproducción

    env -u XDG_CACHE_HOME cargo xtask rust-build-std
