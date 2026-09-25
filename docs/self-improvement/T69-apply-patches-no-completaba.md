# T69 — `dl.rs` se copia al vendor y nadie lo compila

**Origen:** medido en [T39](T39-bootstrap-libstd.md), fuera de su alcance.
**Aplica en:** `config/rust-soso/` · **Estado:** pendiente.

## Lo que se ve

`apply-patches.sh` copia dos cosas a `library/std/src/sys/pal/soso/`:

1. El árbol `config/rust-soso/tree/.../sys/pal/soso/`, cuyo `mod.rs` **no
   declara submódulos** («PAL soso — basada en `unsupported` + `soso_rt`»).
2. Encima, `config/rust-soso/sys/pal/soso/dl.rs`.

El `mod.rs` que queda no dice `mod dl;`, así que ese fichero está en el vendor
y **nada lo compila**. Comprobado en el vendor real:

    $ ls ~/.cache/soso-rust-vendor/library/std/src/sys/pal/soso/
    dl.rs  mod.rs
    $ grep -n "mod dl" .../soso/mod.rs
    (nada)

## De dónde viene

Hay **dos generaciones** de la PAL en el repo:

- `config/rust-soso/sys/pal/soso/` — la antigua. Su `mod.rs` declara
  `pub mod dl; pub mod io; pub mod thread;` y su comentario dice «Copiar este
  árbol a `library/std/src/sys/pal/soso/`».
- `config/rust-soso/tree/library/std/src/sys/pal/soso/` — la nueva, basada en
  `unsupported`, sin submódulos.

El script copia la **nueva** y arrastra un `cp` de `dl.rs` de la **antigua**.
De la antigua, `io.rs` y `thread.rs` no se copian nunca.

## Qué hay que decidir

Una de dos, y hace falta compilar para saber cuál:

1. La PAL nueva **necesita** `dl`, y entonces su `mod.rs` debe declararlo (y
   quizá `io` y `thread` también).
2. No lo necesita, y entonces sobran el `cp` y los tres ficheros de la
   generación antigua.

No se decide aquí porque **hoy no se puede compilar libstd para soso**: falta
el enlazador ([T68](T68-wild-soso-nombre.md) lo dejó medido). Decidirlo a
ciegas es justo lo que hizo que dos generaciones convivieran.

## Mientras tanto

`soso_improve_core::receta::pasos_libstd` traduce el script **fielmente**,
copia incluida, y el paso se llama «PAL: dl.rs (hoy no lo declara nadie —
T69)». Una traducción que arregla cosas por el camino deja de ser una
traducción.
