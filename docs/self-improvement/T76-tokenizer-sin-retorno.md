# T76 — Tests de tokenizer que no pueden pasar en silencio

**Hito:** SI-1 · **Estado:** **hecha** (2026-09-26).
**Dependencias:** [T03](T03-perfil-modelo.md), [T06](T06-render-chat.md).
**Origen:** deuda D6 en [DECISIONES.md](DECISIONES.md).
**Contrato:** convención de pruebas del plan (no secciones C1–C7).
**Seguimiento:** [seguimiento/T76.md](seguimiento/T76.md).

## Problema

Cuatro tests contractuales de render y de petición HTTP hacían `return` si no
encontraban `tokenizer.som`. Libtest los marca como **ok** aunque no ejecutaran
ninguna aserción. Eso es indistinguible de haber comparado tokens o preparado
Q04. El test de los cinco fixtures (`los_cinco_fixtures_coinciden_en_texto_y_tokens`)
ya fallaba con la ruta; estos cuatro no.

## Alcance

- [crates/soso-llm-core/tests/conversation_render.rs](../../crates/soso-llm-core/tests/conversation_render.rs): `otra_familia_devuelve_error`.
- [crates/soso-llm-api/tests/request.rs](../../crates/soso-llm-api/tests/request.rs): `q04_prepare_si_hay_tokenizer`, `modelo_distinto_perfil_404`, `contexto_excedido_422`; alinear `tokenizer_qwen` con el respaldo `tests/self-improvement/reference/tokenizer.som` (mismo criterio que T06).

Fuera de alcance: AVX2 (`gemm.rs`), `asr.rs`, banco con rustc, parser de herramientas.

## Pasos

1. Compartir la resolución de ruta target → reference en `request.rs`.
2. Sustituir `eprintln!` + `return` por `panic!` con las dos rutas posibles.
3. Ejecutar las dos suites de comprobación en host.

## Comprobación

```sh
cargo test -p soso-llm-core --features std --test conversation_render
cargo test -p soso-llm-api --test request
```

Sin pesos GGUF ni QEMU. Con `tests/self-improvement/reference/tokenizer.som`
presente, los cuatro tests ejecutan sus aserciones. Sin ninguno de los dos
`.som`, el fallo nombra las rutas.

## Cierre

- [x] Catálogo, ficha, índice y seguimiento sincronizados.
- [x] D6 apunta el primer punto a T76; SIMD e integración siguen como deuda.

Resumen: [seguimiento/T76.md](seguimiento/T76.md).
