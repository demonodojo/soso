# T77 — Tests SIMD de matvec sin retorno silencioso

**Hito:** SI-1 · **Estado:** **hecha** (2026-09-26).
**Dependencias:** — (deuda D6 en [DECISIONES.md](DECISIONES.md)).
**Contrato:** convención de pruebas del plan (no secciones C1–C7).
**Seguimiento:** [seguimiento/T77.md](seguimiento/T77.md).

## Problema

Tres tests en [crates/soso-llm-core/src/gemm.rs](../../crates/soso-llm-core/src/gemm.rs)
comparaban kernels AVX2 con el camino escalar, pero hacían `eprintln!` + `return`
si la CPU no tenía AVX2+FMA. Libtest los marcaba **ok** sin ejecutar la comparación.
El target soso declara `+avx2,+fma`; un `cargo test` de host normal no.

## Alcance

- Módulo `gemm::tests`: tres tests escalares que siempre corren (`matvec_f32_escalar_ejecuta_producto`, `matvec_q8_0_escalar_ejecuta_producto`, `matvec_q4_k_escalar_coincide_con_dequant`).
- Módulo `gemm::tests_avx2`: compilado sólo con `target_feature = "avx2"` y `fma`; compara `avx2::*` con escalar y falla si el perfil mintió (CPU sin AVX2 en runtime).

Fuera de alcance: kernels de producción, `attn.rs`, `asr.rs`, banco con rustc o pesos.

## Pasos

1. Extraer fixtures compartidos y tests escalares obligatorios.
2. Mover la comparación SIMD a `tests_avx2` bajo `cfg(target_feature)`.
3. Ejecutar las dos suites de comprobación en host.

## Comprobación

```sh
cargo test -p soso-llm-core --features std --lib gemm::
RUSTFLAGS="-C target-feature=+avx2,+fma" cargo test -p soso-llm-core --features std --lib gemm::
```

Sin pesos GGUF ni QEMU. La primera ejecuta los escalares (7 tests en `gemm::` incluyendo rope/residual). La segunda añade los tres de `tests_avx2`.

## Cierre

- [x] Catálogo, ficha, índice y seguimiento sincronizados.
- [x] D6: el punto SIMD apunta a T77; integración rustc/pesos sigue como deuda.

Resumen: [seguimiento/T77.md](seguimiento/T77.md).
