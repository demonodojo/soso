# T78 — Entradas de integración ASR y banco sin ok silencioso

**Hito:** SI-0 / SI-1 · **Estado:** **hecha** (2026-09-26).
**Dependencias:** [T02](T02-banco.md), [T76](T76-tokenizer-sin-retorno.md).
**Origen:** segundo punto de D6 (integración) en [DECISIONES.md](DECISIONES.md).
**Contrato:** convención de pruebas del plan.
**Seguimiento:** [seguimiento/T78.md](seguimiento/T78.md).

## Problema

Tests de **integración** en `asr.rs` y compilación de candidatos en `banco.rs`
hacían `return` si faltaban modelos ASR, el wav de referencia o `rustc`.
Libtest los contaba como **ok** sin ejecutar aserciones.

T76 arregló el tokenizer contractual; este trozo separa host mínimo de perfil
con artefactos pesados o herramientas externas.

## Alcance

- [crates/soso-llm-core/tests/asr.rs](../../crates/soso-llm-core/tests/asr.rs):
  tests ASR marcados `#[ignore]` con mensaje; comprobación explícita al ejecutar
  con `--ignored`; test `asr_entradas_si_perfil_exigido` cuando
  `SOSO_REQUIRE_ASR=1`.
- [tools/soso-improve/tests/banco.rs](../../tools/soso-improve/tests/banco.rs):
  tres tests que compilan candidatos fallan si no hay `rustc`; test
  `banco_rustc_si_perfil_exigido` cuando `SOSO_REQUIRE_RUSTC=1`.

Fuera de alcance: AVX2 (`gemm.rs`, previsto T77), casos/umbrales del banco,
pesos de chat.

## Perfiles

| Perfil | Comportamiento |
|---|---|
| Host mínimo | `cargo test` normal: ASR **ignored** (visible en el resumen); banco compila candidatos solo si hay `rustc`, si no **panic** con ruta clara |
| Integración ASR | `cargo test -p soso-llm-core --test asr -- --ignored` |
| CI ASR | `SOSO_REQUIRE_ASR=1` + `--ignored` en `--test asr`; falla si faltan artefactos |
| CI banco | `SOSO_REQUIRE_RUSTC=1 cargo test -p soso-improve --test banco` |

## Comprobación

```sh
cargo test -p soso-llm-core --features std --test asr
cargo test -p soso-improve --test banco
```

## Cierre

- [x] Catálogo, ficha, índice, seguimiento y D6 sincronizados.

Resumen: [seguimiento/T78.md](seguimiento/T78.md).
