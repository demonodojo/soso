# T82 — Prefill por lotes

**Origen:** destapado el 2026-10-08 al intentar [T22](T22-primera-mejora.md). **Estado:** en curso (2026-10-08): kernels y forward por bloques hechos y verificados; falta medir en el guest y cerrar. Era lo siguiente a hacer (decisión del usuario).

## Por qué

El prefill procesa **un token cada vez** (`prefill_prompt_with` → `embed_token` + `forward_step` por token): cada token lee los 4,8 GB de pesos del 7B. Medido a ~0,55–1 token/s, igual en el host con un hilo (`generar_crudo`) que en el guest con 4 núcleos, así que **no es QEMU**: es ancho de banda de memoria. El prompt de OpenCode (~4 150 tokens) cuesta 65–125 min en el primer turno de cada sesión. [T81](T81-reutilizar-kv.md) evita repetirlo en los turnos siguientes, pero el primero sigue pagándolo y cada experimento de prompt que cambie el prefijo también.

## Qué hacer

Procesar el prompt en bloques de N tokens: para cada capa, leer cada matriz de pesos **una vez** y multiplicarla por los N vectores (la dequantización de cada fila se amortiza), con atención causal dentro del bloque y los K/V apendados en orden. Objetivo: pasar de memoria-limitado a cómputo-limitado (órdenes de magnitud en prefill).

Puntos de partida:
- `crates/soso-llm-core/src/runtime.rs`: `prefill_prompt_with`, `forward_layers_range_clock`.
- `crates/soso-llm-core/src/gemm.rs`: ya hay `matmul_xwt_f32` y los `matvec_q8_0`/`matvec_q4_k`; faltan las variantes multi-vector sobre pesos cuantizados.
- `RowParallel` (pool de hilos del guest, L3b) para repartir filas.

## Requisitos

- **Exactitud**: el resultado (K/V y logits) tiene que ser idéntico al del prefill token a token con la misma suma por fila; se prueba contra el camino actual con los modelos tiny de `tests/generate.rs` y con el 7B real (mismos tokens generados).
- Compatible con T81 (el bloque sólo cubre `prompt[ya..]`) y con cancelación/progreso por bloque.
- No cambiar el decode (token a token, memoria-limitado por naturaleza).
- Medir en el guest con el prompt de 4 146 tokens de `tests/self-improvement/opencode/peticion-01-turno-1-lectura.json` y dejar el tiempo antes/después.

## Cierre

Con el prefill por lotes, repetir [T22](T22-primera-mejora.md): ya se podrá iterar sobre el prompt en minutos en vez de horas.

## Hecho (2026-10-08)

**Diagnóstico corregido.** El plan inicial decía «memoria-limitado: leer cada matriz una vez». El microbenchmark (`crates/soso-llm-core/examples/bench_matvec.rs`, matriz FFN del 7B 18 944×3 584 Q4_K, AVX2) lo desmintió: leer la fila una vez para varios vectores con el mismo kernel da 1,04×. El kernel de un vector es **latencia-limitado**: un solo acumulador por fila (`acc = fmadd(coef, xv, acc)`), ~2 pesos/ciclo (8,07 ms por matriz = 4,5 GB/s, ≈ 0,95 s por token del modelo entero). Lo que acelera es tener **N acumuladores independientes** y dequantizar el bloque una sola vez.

**Kernels por lotes** (`gemm.rs`): `matvec_q4_k_batch[_strided]`, `matvec_q8_0_batch[_strided]` (AVX2 con 1–8 vectores por pasada; fuera de AVX2, bucle del matvec suelto). Cada vector sigue **la misma secuencia de operaciones** que el kernel de un vector → **idéntico bit a bit** (`tests/gemm_batch.rs`, con y sin AVX2, n = 1…19, filas y columnas variadas). Medido: 1,53 ms/vector con 8 vectores frente a 6,58 ms sueltos (**4,3×**); con 16–32 baja a ~3,2× (caché), de ahí el bloque de 8.

**Forward por bloque** (`layer.rs`): `matvec_view_batch_par` (reparte filas con `RowParallel`) y `LayerExecutor::forward_layer_block`: proyecciones y FFN por lotes, atención token a token con el mismo código y K/V añadidos en orden. `Runtime::forward_bloque` + `prefill_prompt_with`: bloques de 8 tokens cuando es seguro (atención clásica con KV f16, FFN denso, sin GPU ni capas remotas, pesos residentes, sin deslizamiento de ventana a mitad de bloque); si no, el camino de siempre. Interruptor `Runtime::prefill_por_bloques`.

**Pruebas** (`tests/generate.rs`): estado tras el prefill —K/V, `hidden`, `pos`— **idéntico bit a bit** por bloques y token a token, en dos modelos tiny, con longitudes 2, 7, 8, 9, 17, 20 y comprobando que se usó el camino nuevo (`tokens_en_bloque`); y mismos tokens generados. El contrato de cancelación se conserva: el observador sigue viendo cada `PrefillToken{done}`.

**Medida preliminar con el 7B real (host, 1 hilo, AVX2):** prompt de 554 tokens + 24 generados en **160,1 s** (`first-improvement/crudo-bloques-a-read.txt`).
