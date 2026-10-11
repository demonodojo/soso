# T82 — Prefill por lotes

**Origen:** destapado el 2026-10-08 al intentar [T22](T22-primera-mejora.md). **Estado:** hecha (2026-10-08), con límites. Era lo siguiente a hacer (decisión del usuario).

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

**Medida con el 7B real (host, 1 hilo, AVX2), mismo prompt de 554 tokens:**

| | Tiempo | Archivo |
|---|---|---|
| sin bloques (`SIN_BLOQUES=1`), 1 token generado | 390,4 s (0,70 s/token) | `crudo-sin-bloques-a-read.txt` |
| con bloques, 24 tokens generados | 160,1 s (prefill ≈ 143 s ≈ 0,26 s/token) | `crudo-bloques-a-read.txt` |

→ **≈ 2,7× en prefill con un hilo.** Menos que el 4,3× del kernel suelto porque la atención, RoPE, normas y SwiGLU siguen yendo token a token (Amdahl); la atención además crece con la posición, así que en prompts largos pesa más. Queda por medir en el guest (4 núcleos, `RowParallel`) y por perfilar la atención.

## Guest: por qué al principio no se notó (2026-10-08)

La primera medida en el guest (prompt de 554 tokens) dio **600 s, igual que antes**: el camino por bloques no se estaba usando. Se añadió un diagnóstico (`Runtime::motivo_sin_bloque`, registrado por el servidor del guest tras cada generación: «prefill N tokens de prompt; por bloques acumulado M; motivo sin bloque: …») y dijo **«H2O activo»**: el planificador del guest usa la política de expulsión H2O (la ventana de KV es más corta que `max_seq`) y mi versión excluía ese caso. Con H2O el KV además cae a **int8** (KIVI-lite).

Arreglo: el forward por bloque hace lo mismo que el de un token también con H2O (acumula la masa de atención por token) y con KV int8 (`append` y la atención genérica ya saben de dtype). Prueba (`tests/generate.rs::bloques_con_h2o_acumulan_la_misma_masa`): planificador con H2O + pesos residentes + KV int8, estado idéntico bit a bit —K/V int8, escalas, **masa de atención**, `hidden`, `pos`—.

**Medida en el guest** (7B, `-smp 4`, 8 GiB; petición `corto/a-read.json`, 554 tokens + hasta 40 generados):

| | Tiempo | Bloques |
|---|---|---|
| antes (token a token) | 600 s (`medida-diag-guest-sin-h2o.txt`) | 0 de 554 |
| prefill por bloques | **210 s** (`medida-diag-guest-con-bloques.txt`) | **554 de 554** |

→ **≈ 2,9×** en el guest, coherente con el 2,7× del host con un hilo. Queda por debajo del 4,3× del kernel porque la atención, RoPE, normas y SwiGLU siguen yendo token a token y porque el reparto de filas entre hilos tiene su coste.

## Límites y siguiente

- **Atención por lotes** (agrupar las cabezas/tokens del bloque) y reparto de la atención entre hilos: la atención crece con la posición y en prompts largos pasa a ser lo que domina.
- El KV int8 + H2O del planificador cambia la numérica respecto al KV f16 del host: el mismo prompt dio salidas distintas en host y guest (`"name": "read"` frente a `"function": "readFile"`), así que **los ajustes de prompt hechos en el host no se trasladan tal cual al guest**.
- El arranque de `soso-llm serve` en el arnés del guest falla de forma intermitente (~2 de cada 3 veces: «serve terminó solo» / «QEMU murió cargando serve»), con 12 GiB casi siempre y con 8 GiB a veces; `first-improvement/lanzar-t82.sh` reintenta. Sin diagnosticar.

## Medida final en el guest: el prompt completo de OpenCode (2026-10-08)

7B, `-smp 4`, 8 GiB, petición `00-original` (4 146 tokens de prompt, 32 generados), `medida-t82-guest.txt`:

| | Tiempo |
|---|---|
| antes de T81/T82 (token a token) | 3 846 s (64 min) |
| **prefill por bloques** (4 146 de 4 146 tokens por bloques) | **1 217 s (20 min)** → **3,2×** |
| petición siguiente (4 171 tokens, prefijo común reutilizado, T81) | **34 s** |

**Lo que no se ha conseguido:** bajar de 20 min el primer turno de cada sesión. Queda la atención por lotes y repartir la atención entre hilos.

**Hallazgo adicional.** En esa segunda petición el 7B del guest escribió ` ```json {"function": "read", "filePath": "TASK.md"} ``` `, un esquema que ni el host (KV f16: `{"name": "read", "arguments": …}`) ni el prompt piden. El guest usa KV int8 y H2O; el host, f16 y sin H2O. Falta aislar si es la cuantización del KV lo que degrada la llamada a herramienta (el prompt es el mismo).

## Atención por grupos GQA y reparto entre hilos (2026-10-10, pasada de optimización 1)

**Por qué.** Con el 14B en el guest el primer turno tardaba ~80 min y sólo se usaban ~1,4 de los 4 núcleos. La
instrumentación por fases (`layer::perfil_activar`, contadores apagados por defecto) y el banco con una capa
sintética del 14B (`examples/bench_bloque`, pesos Q4_K/Q8_0 sintéticos, 4 096 tokens) lo explican: **la atención era el
77 % del tiempo y no paralelizaba** (el matvec sí: 245 GFLOP/s con 4 hilos). Es sólo el 7 % de las operaciones, pero
iba en serie, con un kernel de ~5 GFLOP/s (conversión f16→f32 emulada, una sola cadena de FMA, una cabeza cada vez
y `expf` escalar), y con H2O activo caía además al kernel genérico (×1,8–2,3 más lento).

**Qué se hizo** (todo con el mismo resultado bit a bit entre bloque y token a token):
1. `attn_grupo::attention_f16_group`: las `g` cabezas de un grupo GQA se procesan juntas; K/V se convierten una vez
   por tile; Q·K de 8 en 8 tokens con 8 acumuladores; `exp` vectorial (`simd_math::exp256`, error < 2e-7); F16C si el
   binario lo trae (`user/x86_64-soso-user.json` ahora declara `+f16c`: toda CPU con AVX2 lo tiene). Invariante
   probada (`tests/attn_grupo.rs`): el resultado de una consulta **no depende del tamaño del grupo**.
2. La atención del bloque reparte `n × kv_heads` tareas entre hilos (`atencion_f16_grupos`), y también la
   decodificación (por cabeza KV).
3. H2O sólo acumula masa (y por tanto sólo usa la ruta genérica) cuando la secuencia pasa de la mitad de la ventana
   (`ResourcePlanner::h2o_masas`): antes de eso no puede haber expulsión y la masa no sirve.
4. `swiglu`/`silu` vectoriales (misma `exp256`) y repartidos entre hilos en el bloque.

**Resultado en el banco** (host, 14B, 2 capas, 4 096 tokens, 4 hilos; los guest comparten CPU con el banco, ±15 %):

| | Total | Atención | swiglu | matvec (GFLOP/s) |
|---|---|---|---|---|
| antes | 98,8 s | 76,4 s (77 %) | 3,8 s | 245 |
| + grupos GQA/F16C, atención repartida | 25,3 s | 3,2 s | 3,8 s | 249 |
| + swiglu vectorial y repartido | **21,9 s** | 3,2 s | 0,016 s | 243 |

→ **4,5×** en el banco. Con 1 hilo: 138,8 s → 72,2 s (sin repartir, sólo el kernel).

### Pasadas 2 y 3 (2026-10-10)

| Pasada | Qué | Efecto en el banco (14B, 4 096 tokens, 4 hilos) |
|---|---|---|
| 2 | `swiglu`/`silu` vectoriales (`simd_math`) y repartidos entre hilos | 3,8 s → 0,016 s |
| 3a | La atención del bloque comparte la conversión de K/V entre **4 tokens × cabezas del grupo** (`attention_f16_tokens`); cada consulta da **lo mismo que sola** (prueba `un_bloque_de_tokens_da_a_cada_consulta_lo_que_daria_sola`) | atención 3,2 s → 2,8 s |
| 3b | Reescritura del kernel: puntuaciones por transposición `hadd` (8 sumas horizontales → 1), máximo y `exp` sin salir de registros, axpy de V en dos mitades de 8 acumuladores (caben en los 16 registros) | 2,8 s → 1,7–1,9 s (≈ 50 GFLOP/s por hilo; aislado 82 por token-cabeza) |
| 3c | RoPE con tabla de cos/sin **una vez por token** (antes `powf`/`sinf`/`cosf` por cabeza: 48 veces por token en el 14B); idéntico bit a bit (`rope_cabezas_es_identico_a_rope_por_cabeza`) | rope+kv 0,96 s → 0,13 s |

Total del banco: **98,8 s → ~21 s (4,7×)**; ya no queda nada serie relevante (atención 9 %, rope+kv 0,6 %, resto < 0,5 %) y el
matvec es el **90 %**.

**Pruebas de que no se rompe nada:** las 18 suites de `soso-llm-core` con y sin `-C target-feature=+avx2,+fma,+f16c`,
`soso-llm-api` y la prueba `bloques_con_h2o_*` (adaptada: la masa de H2O sólo se acumula pasada la mitad de la ventana;
hay una variante con KV f16 que mezcla en un mismo bloque el kernel rápido y el genérico con masa). El 7B real sigue
escribiendo `{"name": "read", …}` con el prompt corto, con y sin planificador.

**Intentado y descartado.**
- **Bloques de 12 tokens** en vez de 8 (`q4_k_nb::<12>`): más lento (2,30 ms/vector frente a 1,83): se derrama a pila porque
  AVX2 sólo tiene 16 registros. 8 es el óptimo.
- **Desempaquetar las escalas de Q4_K por palabras de 32 bits**: como mucho ~5 % del kernel; no compensa.
- **Producto entero Q4_K × activaciones Q8_K** (2026-10-11): la estimación inicial de µops no contaba `_mm256_maddubs_epi16`.
  Implementado en `crates/soso-llm-core/src/q8k.rs`; **`Q4_Q8K_EN_PRODUCCION` sigue en `false`** (bench ~1,0× vs lote f32 en la
  FFN sintética del 7B, 2026-10-11; listón 1,8×). Pruebas: `tests/gemm_batch.rs::q4_k_q8k_*`; medir con `bench_matvec` + AVX2.

**Lo que queda (matvec ≈ 90 %).** El kernel de matvec por bloques (`q4_k_nb`, `q8_0_nb`) rinde ~60 GFLOP/s por hilo, cerca
del techo de un diseño en coma flotante con AVX2 y no es limitado por memoria (`bench_matvec`: 1,83 ms/vector con n = 8 y
2,0 con n = 16/32).
