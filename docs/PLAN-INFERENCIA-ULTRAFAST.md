# Plan: incorporar DeepSeek Ultrafast a la inferencia de soso

> **Revisión de viabilidad:** las fases siguientes son propuestas, no resultados
> medidos. Aplicar las correcciones y puertas de decisión de la sección 9 antes
> de implementarlas; prevalecen sobre las estimaciones y cierres anteriores.

**Fecha:** 2026-11-15 · **Origen:** [basetenlabs/deepseek-ultrafast](https://github.com/basetenlabs/deepseek-ultrafast)
(motor Rust + kernels CUDA para DeepSeek-V4.1-Flash en sm_100a/sm_103a).

**Licencia del origen:** Elastic License 2.0 — *source-available*, no MIT/Apache.
Permite usar, copiar, modificar y distribuir, pero **no** ofrecer el software como
servicio gestionado ni quitar avisos. Por eso este plan **reimplementa ideas y
contratos**, y no copia ficheros de DSUF: si alguna vez se copia código literal,
entra con su aviso y sus términos.

**Objetivo:** llevar a `soso-llm-core` / `soso-llm-api` las ideas de ese motor que
se pueden pagar en el hardware de soso, en fases que cierren con una medición y
un test host reproducible, sin copiar su stack de centro de datos.

**No es objetivo:** ejecutar DeepSeek-V4.1 ni replicar sus números (1.740 tok/s
con 4×GB300, prefill de 76-85k tok/s, 1M de contexto). soso corre en una GPU de
consumidor de 8-16 GiB, en guest sin GPU, y su clúster son 1-2 nodos por TCP.

---

## 1. Qué hace DSUF, con evidencia en su código

| # | Mejora | Cómo está hecha | Dónde |
|---|--------|-----------------|-------|
| 1 | Decode especulativo por **bloques verificados** | Cada paso redacta un bloque (hasta 8 tokens) y lo verifica en **un** forward de `k+1` posiciones; 5,6 tokens aceptados por paso de media. La contabilidad separa *emitido* de *comprometido* y de *aceptado* para no publicar el lookahead ni confirmar KV de tokens rechazados | `engine/src/decode_cache.rs` (`boundary`, `tokens`), `engine/src/bin/run.rs` |
| 2 | Borradores baratos y varios | Dos cabezas de borrador: una **MTP** (8 posiciones) y una **tabla Markov** cuantizada con selección top-256 en un solo kernel, ambas conscientes del muestreo (`temp`, top-p) | `kernels/cu/draft_head8.cu`, `kernels/cu/draft_markov.cu` |
| 3 | Muestreo y verificación **en GPU** | El kernel de sampling hace top-k, muestreo y el rechazo del borrador; al host solo baja el token. El repo trae un test **chi²** que compara la distribución del decode especulativo con el exacto (`--samp-test`) | `kernels/cu/sample.cu`, `run.rs` |
| 4 | KV en **FP4** | Cuantiza K/V a 4 bits por grupo con escalas; el empaquetado valida finitud y rango con un flag de validez leído por el host antes de publicar el snapshot | `kernels/cu/kv_fp4.cu`, `engine/src/fp4.rs` |
| 5 | Snapshot de KV **contenido-direccionable** | Manifiesto con versión, *fingerprint* del perfil, tokens, segmentos, bytes y `blake3`; **ventana de cola** (`SAVED_TAIL=8192`, `TAIL_READ=256`) para que el snapshot valga aunque el decode siga avanzando; export asíncrono a *pinned host* con evento | `kv-transfer/src/snapshot.rs`, `engine/src/kvstore.rs` |
| 6 | Reutilización **pull** entre réplicas | Redis guarda qué réplica posee el KV de cada conversación; si el turno cae en otra, esta **trae** el snapshot por RDMA en vez de recalcular el prefijo. Elimina el enrutado por prefijo | `kv-transfer/src/{directory,rdma,lib}.rs` |
| 7 | Kernels **AOT por arquitectura** | `build.rs` compila cada `.cu` a cubin con `nvcc -arch=$DSUF_SM`, lo incrusta con `include_bytes!`, descubre los símbolos leyendo las secciones `.text.*` del ELF y carga **variantes** sólo si el target es sm_103a | `kernels/build.rs`, `kernels/src/lib.rs` |
| 8 | *Staging* asíncrono de expertos | Doble búfer *pinned* + stream propio para solapar HtoD con cómputo; si no hay pinned, cae a *pageable* y lo registra como diagnóstico | `engine/src/weight_staging.rs` |
| 9 | **Engram** | Memoria n-grama condicional por capa: vocabulario comprimido (129.280), primos por *bucket*, multiplicadores hash sembrados por capa, n-gramas *padded* | `engine/src/engram.rs`, `model/src/config.rs` |

Las nueve piezas son de fases distintas: 1-3 y 5-8 son **algoritmos y contratos**
que se pueden portar hoy; 4 depende de tensor cores FP4 (sm_100a+) pero su forma
cuantizada es portable; 9 es **arquitectura del modelo** y no se puede añadir a
los modelos que carga soso.

---

## 2. Dónde está soso hoy (para no rehacer lo hecho)

| Pieza | Estado en soso | Ruta |
|-------|----------------|------|
| Decode especulativo | Sí: PLD n-grama con autotune de `n` y tope de borrador, verificación por bloque (`borrador_por_bloques`) y política de longitud por coste | `crates/soso-llm-core/src/{plan,runtime,especulativo}.rs` |
| Exactitud del especulativo | Greedy y muestreo con temperatura en bloques sin restricciones; falta validación estadística y de GPU | `runtime.rs::especular_en_bloque` |
| KV | f16 o **int8** por token (KIVI-lite) + expulsión H2O; KV residente en GPU con rebobinado por `pos0` | `kv.rs`, `cuda/capa.rs` (`kv_preparar`) |
| Atención de decode | Tiled/online-softmax en CPU (AVX2) y kernel de atención con KV residente en GPU | `attn.rs`, `cuda/capa.rs` |
| Weights | MXFP4/Q4_K/Q8_0 en CPU y GPU; MoE con gate/up Q4_K y down Q8_0 | `gemm.rs`, `layer.rs`, `moe_gpu.rs`, `cuda/capa/moe.rs` |
| Muestreo | En host; por token baja la fila de logits completa por PCIe y sube la fila de embedding | `cuda/decode.rs` (DtoH de `LOGITS`), `sample.rs` |
| Snapshot de KV | Sólo prefijo **de CPU** para failover (`KevPrefixSnapshot`); el KV de GPU no se persiste | `kev.rs`, `runtime.rs` |
| Clúster | Pipeline por capas en estrella, journal con `prefix_hash`, transporte TCP | `pipeline.rs`, `cluster/{nodo,estrella}.rs` |
| Kernels GPU | 3 PTX versionados (`.target sm_80`); fuentes `.cu` y cabeceras en `lxdde/ports/nouveau`, AOT experimental opcional | `crates/soso-llm-api/cuda/*.ptx`, `cuda.rs` |

Conclusión: el hueco real no es «añadir especulación» (ya la hay) sino
**(a)** hacerla exacta con muestreo, **(b)** dejar de mover logits y filas K/V al
host en cada paso, **(c)** persistir y compartir el KV, y **(d)** compilar kernels
por arquitectura en vez de versionar PTX de sm_80.

---

## 3. Criterios de adaptación

1. **Se copia el invariante, no el número.** DSUF mide 5,6 tokens/paso con 4-8
   GPU y un modelo V4.1; en soso el objetivo es más tokens aceptados por paso
   con el mismo KV y sin degradar la distribución.
2. **Toda fase cierra con medición.** `cargo run --release -p soso-llm-core
   --features std --example hostrun` y `scripts/l6-h-bench.sh` son la vara; sin
   número antes/después la fase no se cierra.
3. **Nada de dependencias de centro de datos.** Redis → fichero en sosofs; RDMA →
   TCP (y RDMA sólo si algún día hay hardware, tras un `check_rdma` equivalente);
   variantes de kernel no portables → fuera.
4. **El guest manda.** Lo que aterrice en `soso-llm-core` debe compilar
   `--no-default-features`; si necesita `std`, va detrás de feature como el resto.
5. **Reimplementar, no copiar.** La licencia Elastic 2.0 del origen hace que el
   trasvase sea de diseño (fórmulas, invariantes, estructura de manifiesto), no de
   ficheros; las ideas no se licencian, el código sí.

---

## 4. Fases

### F0 — Línea base medida (1-2 días)

Sin esto no se puede afirmar ninguna mejora.

**Avance:** `hostrun` informa el tiempo hasta el primer token y los contadores
de borrador, y calcula el rendimiento con tokens generados, sin contar el prompt.
El tiempo total incluye prefill; todavía no es una medida aislada de decode.
Faltan escenarios reproducibles y mediciones antes/después; F0 sigue abierta.

**Seguimiento 2026-11-15:** `hostrun` emite `medicion:` en JSON con prefill
hasta `BeforeDecode`, TTFT, percentiles entre callbacks, forwards de decode y
contadores de borrador. `SOSO_NO_DRAFT=1` permite comparación. Los intervalos
incluyen la salida por consola y los tokens de un mismo bloque pueden llegar
juntos; no equivalen a latencia de kernel. Comprobación de funcionamiento en
host Intel Core Ultra 7 255HX, `tiny-q4k` sintético, prompt 29 tokens, greedy,
32 nuevos, build release: 17 forwards y 15/15 borradores aceptados frente a
32 forwards sin borrador; secuencias iguales. Dos procesos concurrentes, una
pasada, ~42,5 y 43,3 ms totales: **no son una comparación de rendimiento**.
Comandos: `target/release/examples/hostrun target/tiny-q4k-model 'hola hola
hola hola hola hola' 32` y el mismo con `SOSO_NO_DRAFT=1`. Compilación de
`hostrun` release, `cargo check -p soso-llm-core --no-default-features --offline`
y `cargo test -p soso-llm-core --features std --lib generation::tests --offline`
terminaron con código 0. `nvidia-smi` devolvió 9 por falta de controlador;
queda pendiente validar CUDA y placa. Próximo paso: pruebas secuenciales y
repetidas con modelo real, condiciones de memoria controladas y cálculo de
throughput de decode y medición de transferencias.

**Servicio host:** `SOSO_BENCH_JSON=1` en `serve_host` emite en stderr una
línea `[soso] medicion:` por petición con JSON: tokens del prompt, prefijo KV
efectivamente reutilizado, tokens procesados en prefill, TTFT, throughput y
percentiles entre entregas, duración total y deltas de forwards/borrador. Los
tiempos incluyen la entrega al cliente; no son tiempos de kernel. No modifica
la respuesta HTTP. Pruebas host: `cargo test -p soso-llm-api --features std
--lib tests_medidor --offline` (1/1), `cargo test -p soso-llm-core --features
std --test generate prefijo --offline` (5/5), y `cargo test -p soso-llm-api
--features std --test host_service --offline` (5/5; fuera del sandbox porque
necesita sockets locales). Falta incorporar estas métricas al banco reproducible
y medir bytes de transferencia; no se cierra F0.

**Transferencias del decode CUDA:** con `SOSO_BENCH_JSON=1`, el servidor
incluye `cuda_decode_transferencias`: pasos de decode residente y bytes de
embedding y control hacia GPU, logits y filas KV hacia host. Son deltas por
petición y cuentan copias CUDA explícitas exitosas. No incluyen prefill,
pesos, expertos, copias asíncronas ni tráfico dentro de grafos, por lo que no
son el total HtoD/DtoH de la petición. El cálculo de deltas se probó en host y
compilan `serve_host` y el guest sin `std`; `nvidia-smi` devuelve código 9
porque el controlador no está operativo, así que falta probar los contadores
en una GPU real y completar la instrumentación del resto de las transferencias.

**Banco host CPU:** compilar `cargo build --release -p soso-llm-core --features
std --example hostrun --offline` y ejecutar `python3 scripts/bench-inferencia.py
target/tiny-q4k-model --rondas 5 --calentamiento 1 --salida
target/bench-ultrafast/tiny-q4k.json`. Alterna el orden de las variantes,
exige igualdad de IDs incluso en calentamiento, registra mediciones individuales,
medianas, parámetros, hash de ejecutable/manifiesto/índice e inventario de
shards. El inventario no certifica el contenido de los pesos. Limita cada
proceso por tiempo y rechaza por defecto modelos que superen el 70 % de memoria
disponible (hostrun copia los shards). No compara modelos ni entornos distintos.

En Intel Core Ultra 7 255HX, fixture sintético `tiny-q4k` de dos capas, prompt
de 29 tokens y 32 generados, una pasada de calentamiento, cinco rondas, sin
planificador: mediana de decode 1808 tok/s con borrador y 1770 sin él; 17 frente
a 32 forwards, y 15/15 tokens propuestos aceptados en cada ronda. Con
`SOSO_PLANNER=1`, temperatura 0,8 y tres rondas: 1709 frente a 1773 tok/s,
18 frente a 32 forwards de mediana. Estos ejemplos comprueban el banco; son
demasiado pequeños y sensibles al ruido para justificar una decisión de
rendimiento. Artefactos en `target/bench-ultrafast/`, comando adicional:
`SOSO_PLANNER=1 python3 scripts/bench-inferencia.py target/tiny-q4k-model
--rondas 3 --calentamiento 1 --temperatura 0.8 --salida
target/bench-ultrafast/tiny-q4k-planificado.json`. Ejecutable SHA-256
`fc28369b378c3051643e7ad52fd06546b0cd0243050260c1c78b8d4c7159ba67`;
manifiesto `a89eaf44c6293db1ff64cf3764b27ef70cf252ada5703179b7dbfaddf57c8b1f`,
índice `975c53da96495c105e4f8ee9b8a8d44e6641ae621d496f6b97ddd0a13cf26148`.

- Instrumentar por paso: `ms/paso`, tokens/paso, aceptación por posición (ya hay
  `borrador_ofrecidos`/`borrador_aceptados` en `runtime.rs` y `pld_accepted`/
  `pld_attempts` en `plan.rs::PlannerStats`), `DtoH`/`HtoD` por token y bytes.
- Añadir el desglose al ejemplo `hostrun` y a un `--json` de `soso-llm-api` para
  que `scripts/l6-h-bench.sh` lo consuma.
- Fijar 3 escenarios reproducibles: (i) 7B denso Q4_K en host, (ii) MoE tipo
  Qwen3 en GPU con offload, (iii) 2 nodos QEMU por TCP.

**Cierre:** tabla de línea base en este documento con GPU, modelo, quant, longitud
de prompt y `tok/s`; guardada también en `docs/ESTADO.md` sólo como enlace.

### F1 — Especulación exacta con muestreo (1-2 semanas)

El cambio conceptual más importante del repo y el que soso no tiene.

**En curso (2026-11-15).** Para PLD determinista y sin restricciones, el
verificador por bloques muestrea directamente de la distribución objetivo en
cada posición y conserva la primera muestra que no coincide como token
pendiente. Esta variante de acoplamiento es equivalente al rechazo con q
puntual: acierta con probabilidad p(d) y el resto conserva la distribución
residual. Consume las mismas muestras que la ejecución sin borrador. Si no se
puede verificar por bloques o hay restricciones, el muestreo usa el camino
previo. Las pruebas sintéticas comparan secuencia, informe y estado del KV en
30 semillas con top-k/top-p, y la ruta planificada con paradas y estado RNG.
Faltan mediciones con modelo real y evidencia de GPU. No se declara F1
cerrada ni se atribuye una mejora de rendimiento sin esas comprobaciones.
Evidencia host: `cargo test -p soso-llm-core --features std --test generate
--offline` (28/28, código 0), `cargo test -p soso-llm-core --features std
--test generation_cancel --offline` (8/8, código 0) y `cargo check -p soso-llm-core
--no-default-features --offline` (código 0). Pruebas adicionales cubren la
cancelación dentro del bloque, restauración del KV y reutilización del runtime,
y límites de contexto y presupuesto con temperatura. El estado oculto y KV
parcial se restauran también si falla el forward. Falta comparar rendimiento con
un modelo real. La ruta queda habilitada cuando ya está habilitado el borrador;
`borrador_activo = false` conserva el camino anterior.

**Oráculo estadístico:** `tests/especulativo_exacto.rs` construye un modelo
f32 pequeño cuya distribución teórica independiente por token es 0,5/0,3/0,2.
Para 20 000 semillas mezcladas compara cada secuencia de dos tokens con y sin
borrador y contrasta las frecuencias contra la distribución teórica, con
aceptaciones y rechazos efectivos. En release, `top_p=1` produjo χ²=6,724
(8 grados; umbral 32), 10 029 propuestas y 4 990 aceptadas; `top_p=0,7`
produjo χ²=0,667 (3 grados; umbral 22), 12 508 propuestas y 7 786 aceptadas.
Comando: `cargo test --release -p soso-llm-core --features std --test
especulativo_exacto --offline --locked -- --nocapture` (código 0). El oráculo
no sustituye pruebas de calidad con otros modelos ni valida GPU. Al ejecutar
esta prueba se corrigió también la reserva Q8_K del prefill para anchos f32 que
no son múltiplos de 256.

- Portar la contabilidad de `decode_cache.rs` a `runtime.rs`: separar
  `emitido` / `comprometido` / `aceptado`, con las tres invariantes del original
  (no emitir lookahead, no comprometer token rechazado, no exceder `max_seq`).
  Añadir a `generation.rs` un `StopReason` que distinga rechazo del borrador de
  presupuesto agotado (hoy `Limit`/`ContextLimit` no dicen si se paró por el
  borrador o por el tope), y un `GenerationReport` con `emitido`/`comprometido`.
- Verificación con **rechazo**: aceptar el token `d` del borrador con
  `min(1, p(d)/q(d))` y, al rechazar, muestrear del residual `(p-q)+`. Como el PLD
  es determinista, `q` es una masa puntual y el caso se reduce a aceptar con
  `p(d)` y remuestrear del residual; el resultado es **estadísticamente idéntico**
  al decode sin borrador.
- Puerto del arnés chi² de `run.rs --samp-test` como test de integración:
  `crates/soso-llm-core/tests/especulativo_exacto.rs`, con semilla fija y dof por
  vocabulario.
- Recorte de KV al prefijo aceptado en el camino GPU (`cuda/capa.rs::kv_preparar`
  ya rebobina; falta la ruta de «avanza menos de lo redactado»).

**Cierre:** con `temp>0` y top-p, la distribución de 2 pasos del decode con
borrador pasa el chi² (p>0,01) en 20 semillas; `kv.rs` queda en `pos0 + aceptados`;
`cargo test -p soso-llm-core --features std` verde.

### F2 — Muestreo y verificación fusionados en GPU (1 semana)

- Mover `sample.rs` y la verificación del borrador a un kernel (`sample.cu` en el
  nuevo esquema de F6): top-k, temperatura, aceptación y residual en un lanzamiento.
- Que el decode de `cuda/decode.rs` **no baje la fila de logits**: hoy hay DtoH de
  `vocab × 4` bytes y de las filas K/V por token. Bajar sólo el id elegido y, en
  modo diagnóstico, los primeros `n` ids aceptados.
- Mantener el KV en dispositivo entre pasos (ya ocurre en `gemma_capa`); el camino
  CPU conserva su muestreo actual.

**Cierre:** cero DtoH de logits en decode (`grep` del buffer en el trazado), menos
sincronizaciones por paso, `ms/paso` medido antes/después en el escenario (ii);
`crates/soso-llm-api/tests/{cuda_matvec,cuda_capa,gemma_paridad}.rs` en verde.

### F3 — KV de 4 bits por grupos (1-2 semanas)

**Exploración (2026-11-15):** `kv_fp4.rs` implementa un códec experimental
E2M1 por grupos de 32 con escala f32 y 16 bytes de datos (20 bytes por grupo),
validación de tamaño, finitud, escala y relleno antes de decodificar. Su formato
no es el MXFP4 sintético de pesos. `SOSO_KV_FP4_ANALIZAR=1` en `hostrun` analiza las filas f16 de KV
ya generadas sin alterarlas y muestra bytes y error. En `tiny-q4k` sintético,
prompt «hola hola hola hola hola hola», 32 tokens: 62 464 valores, 124 928
bytes f16 frente a 39 040 bytes FP4 (incluidas escalas), RMSE relativo 7,57 %,
error absoluto máximo 130,17 sobre magnitud máxima 782; cero capas descartadas.
La prueba no mide calidad de respuesta ni predice el error de un modelo real.
El códec pasa `cargo test -p soso-llm-core --features std --lib kv_fp4
--offline` (4/4) y `cargo check -p soso-llm-core --no-default-features
--offline`. Próximo paso: banco con KV de modelo real, error por capa y
comparación de calidad antes de activar el formato. F3 sigue abierta.

**Integración CPU experimental:** `KvDtype::F4` almacena filas por grupos y la
atención genérica lee rangos sin reservar una fila temporal. `hostrun` lo activa
solo con `SOSO_KV_FP4=1` para GQA denso sin ventana ni planificador; f16 sigue
por defecto. Una entrada no finita marca fallo y aborta el paso. Se han probado
recorte, expulsión, H2O, rechazo de NaN, prefill por bloques y atención frente a
un softmax independiente de filas decodificadas. En `tiny-q4k` sintético, con
29 tokens de prompt y 32 de salida, los IDs generados coincidieron con f16;
no demuestra calidad en modelos reales. `cargo test -p soso-llm-core --features
std --test generate kv_fp4_generacion --offline --locked`, `cargo test -p
soso-llm-core --features std --test attn_rutas atencion_fp4 --offline --locked`,
`cargo test -p soso-llm-core --features std --lib kv --offline --locked`
(13/13) y `cargo check -p soso-llm-core --no-default-features --offline --locked`
pasaron. El kernel GPU, modelos reales y comparación de calidad siguen pendientes.

- `kv.rs`: nuevo `KvDtype::F4` con escala por grupo de 32 (e2m1) manteniendo `I8`
  como opción; el dequantizado entra en `attn.rs` y en el kernel de atención.
- Empaquetado con **flag de validez**: si aparece NaN/inf o un valor fuera de
  rango, el snapshot no se publica (contrato de `fp4.rs`/`kvstore.rs`).
- Test de error: paridad de top-1 contra f16 en un conjunto fijo de prompts y cota
  de error por capa.

**Cierre:** bytes de KV por token ÷4 frente a f16 (÷2 frente a `I8`), paridad de
top-1 en el 100 % de un banco fijo, y `cargo test -p soso-llm-core --features std
-- kv:: attn::` verde.

### F4 — Snapshot de KV reutilizable en sosofs (2-3 semanas)

La mejora con más impacto para un SO con `/models` persistente y un servidor local.

**En curso (2026-11-15):** `kvsnap.rs` y `Runtime::{exportar_kv,importar_kv}`
permiten intercambiar en memoria un prefijo completo y comprometido, por ahora
solo GQA f16 en CPU, sin filas expulsadas ni H2O. La imagen incluye versión,
identidad de 32 bytes aportada por el consumidor, geometría por capa, tokens,
estado oculto y SHA-256 de integridad. Se comprueban límites, identidad,
geometría, tokens y finitud antes de reemplazar el estado. El checksum detecta
corrupción accidental; no autentica un origen que pueda modificar la imagen.
El llamante **debe** construir la identidad con el contenido real de los pesos,
tokenizador y configuración: el hash actual de `index.som` no basta. La prueba sintética
exporta, importa y compara la continuación con un runtime en frío, y rechaza
imagen truncada, corrupta o de perfil incorrecto sin mutar el runtime.
Validación: `cargo test -p soso-llm-core --features std --lib kvsnap --offline
--locked` (3/3), `cargo test -p soso-llm-core --features std --test generate
snapshot_kv --offline --locked` (1/1), `cargo check -p soso-llm-core
--no-default-features --offline --locked` y, desde `user/`, `cargo check
--locked --offline -p soso-llm` (código 0). Siguiente: identidad fuerte de los
shards y consumidor persistente con publicación atómica y límites de espacio;
medir también el coste de copias durante exportación e importación.

**Avance adicional host:** `identidad_kv::calcular` calcula SHA-256 sobre el
manifiesto, índice, tokenizador y cada shard referenciado, además de una
identidad de ejecución obligatoria aportada por el llamante. Detecta cambios
durante la lectura y rechaza rutas o enlaces que salgan de `shards`; el modelo
debe permanecer inmutable durante la inferencia. `kvstore_host` proporciona
lectura acotada y escritura temporal, sincronización y renombrado atómico; el
llamante debe proveer un directorio privado por identidad. Las pruebas cubren
cambios de pesos/configuración, rutas inválidas, sustitución de archivo y
corrupción en disco. `cargo test -p soso-llm-api --features std --lib --offline
--locked`: 10/10; `cargo check -p soso-llm-api --no-default-features --offline
--locked`: código 0.

**Integración host:** `serve_host --kv-cache-dir DIRECTORIO
--kv-cache-max-mib N` activa checkpoints CPU para una sola identidad;
requiere directorio preexistente propio y privado (sin bits de acceso para grupo
u otros). El límite es 256 MiB por defecto. La identidad incluye hash del
ejecutable, arquitectura, AVX2/FMA, hilos y contenido del modelo. El modelo debe
permanecer inmutable mientras se ejecuta el servidor. Solo se importa si el
prefijo es mejor que la caché en memoria; los errores caen al prefill normal y
la cancelación no publica. La opción es explícita y no proporciona aislamiento
entre los clientes que comparten la clave de API: todos deben pertenecer a una
misma identidad. La imagen puede contener datos de conversación. La prueba
`SOSO_TEST_MODEL=$PWD/target/tiny-model cargo test -p soso-llm-api --features
std --test kv_cache_real --offline --locked -- --ignored` acredita reinicio,
continuación igual a frío y recálculo tras corrupción con modelo sintético;
no acredita un reinicio de soso ni GPU. El almacén admite 64 imágenes con una
cuota total, deduplica por contenido y selecciona el mayor prefijo válido. Al
agotarse la cuota conserva los archivos existentes y deja de guardar: no hay
evicción automática. Un candado no bloqueante coordina escritores cooperantes;
los temporales abandonados cuentan en la cuota. La selección lee y valida las
imágenes secuencialmente. Un archivo corrupto con el mismo nombre se puede
reemplazar de forma atómica si la cuota permite mantener ambos durante la
escritura; en caso contrario se conserva el anterior. Las métricas del servicio
informan de `cache_kv.lectura_s`, `escritura_s` y `restaurada` por petición.
Una pasada de integración release con `target/tiny-model` reutilizó 9 de 11
tokens, con lectura/validación de 0,082 ms y prefill de 4,33 ms frente a
6,59 ms en frío; es una comprobación sintética aislada, no una comparación de
rendimiento representativa. Las copias, cargas reales y escalado con varias
instantáneas aún requieren medición.
Faltan una identidad de usuario verificable, el camino nativo sosofs, soporte GPU/GDN y
medición del coste de lectura y copias. F4 sigue abierta.

- Tipo `Manifest` en `soso-llm-core` (nuevo `kvsnap.rs`): versión, *fingerprint*
  del perfil (modelo + quant + dtype de KV), tokens, `tail_start`, segmentos,
  bytes y hash. Usar el hash que ya exista en el árbol (CRC del kernel) en vez de
  añadir `blake3` a `no_std`; el `blake3` de DSUF se queda en su host.
- Export asíncrono: búfer *pinned* + evento (`cuda.rs` necesita un
  `cuMemHostAlloc` si no lo tiene) y ventana de cola de 8.192 tokens con 256 de
  relectura, para que el snapshot siga siendo válido mientras el decode avanza.
- Persistencia en `/var/kv/<fingerprint>/<hash>` y reutilización en
  `soso-llm-api/src/service.rs` por conversación; validación estricta en carga
  (versión, fingerprint, tokens, longitud) con rechazo y recálculo, nunca `Err`
  que aborte la inferencia.
- `KevPrefixSnapshot` (CPU) pasa a ser un caso particular del mismo manifiesto.

**Cierre:** segundo turno de la misma conversación (y tras reiniciar el servidor)
procesa **0 tokens de prefill** para el prefijo común; manifiesto con fingerprint
o versión distintos se rechaza con test negativo; snapshot de un modelo distinto
no se aplica.

### F5 — Directorio de propiedad y traída *pull* (2 semanas)

- `cluster/directorio.rs`: prefijo → (nodo propietario, revisión, lease), con
  expiración y `MAX_LEASES`/`MAX_ENTRIES` como el original. En soso el directorio
  es un fichero en sosofs, no Redis.
- `pipeline.rs`: ante un turno cuyo KV vive en otro nodo, **traer** el snapshot en
  vez de enrutar por `prefix_hash`; el `prefix_hash` actual queda como validación
  de coherencia, no como criterio de enrutado.
- Checkpoint por turno: si el snapshot nuevo comparte menos prefijo que el
  anterior, conservar ambos (regla `checkpoint_for`).

**Cierre:** con 2 nodos, mover una conversación al nodo B no recalcula prefill
(tokens de prefill = 0, bytes transferidos registrados); leases y revisiones
caducadas se rechazan; test host sin QEMU para el directorio y E2E con
`cargo xtask test-distributed-llm`.

### F6 — Kernels AOT con variantes por arquitectura (1-2 semanas)

**En curso (2026-11-15):** las tres fuentes y sus cabeceras ya existían en
`lxdde/ports/nouveau`; la afirmación anterior de que faltaban era incorrecta.
`scripts/build-cuda-aot.py --arch sm_86 --include DIR_CUDA --include DIR_CRT`
compila los tres cubins con NVRTC sin GPU y genera un manifiesto con hashes de
fuentes y cubins. `SOSO_CUDA_AOT_DIR=DIR SOSO_CUDA_AOT_SM=sm_86 cargo build
-p soso-llm-api --features std --example serve_host` los incorpora de forma
opcional, comprobando hash, arquitectura ELF y símbolos frente al PTX. En
ejecución, `SOSO_CUDA_AOT=1` solicita AOT solo si la capacidad coincide
exactamente; ante incapacidad de consulta o rechazo de carga usa PTX. Sin las
variables de compilación se mantiene el build previo con PTX.

En este host NVRTC 12.6 compiló los tres cubins sm_86 sin GPU (se usaron las
cabeceras CUDA runtime y CRT de Triton instaladas). La compilación AOT del
servicio y dos tests de selección/respaldo pasaron; se rechazó un cubin truncado
por hash, otro con símbolos ausentes aun con hash actualizado y una arquitectura
incorrecta. `cargo check --offline --locked -p
soso-llm` desde `user/` pasa. NVRTC 12.6 no admite sm_120; no se han medido
arranque ni rendimiento y tampoco se ha ejecutado ningún cubin en GPU. Los
símbolos y hashes no sustituyen la validación numérica o de ABI en hardware.
La elección AOT permanece desactivada por defecto; F6 sigue abierta para
versiones y arquitecturas adicionales, comprobación de ABI y medidas.

- Convertir los 3 PTX versionados (`matvec_q4_0`, `matvec_iq`, `gemma_capa`) a
  fuentes `.cu` en el árbol + `build.rs` con `SOSO_SM` (por defecto sm_80; sm_120
  para GB205, sm_86 para GA107), `nvcc -cubin`, `include_bytes!` y descubrimiento
  de símbolos leyendo `.text.*` del ELF — el patrón de `kernels/src/lib.rs`.
- Variantes por arquitectura: las rutas tensor-core para sm_120 sólo si el target
  lo permite; fallback al cubin base con el mismo nombre de función.
- Si no hay `nvcc`, mantener el PTX actual como respaldo (build no rompe).

**Cierre:** `cargo build` sin `nvcc` sigue funcionando (respaldo), con `nvcc`
incrusta cubin por arquitectura; arranque más rápido y `ms/paso` medido; los tests
de paridad de kernels pasan sin cambios.

### F7 — Staging asíncrono de expertos (1 semana)

- En `cuda/capa/moe.rs` / `moe_gpu.rs`: doble búfer *pinned* con stream propio para
  solapar el HtoD del experto `n+1` con el cómputo de `n`; si no hay *pinned*, caer
  a *pageable* con un diagnóstico único (no por tensor).
- Se apoya en el prefetch MoE especulativo que ya existe en `plan.rs`
  (`note_pld_*`, hint del router anterior).

**Cierre:** en el escenario (ii), `ms/paso` menor con el mismo techo de VRAM;
sin cambio de resultado numérico.

### F8 — Engram: no aplicable, con puerta abierta

No se implementa: exige un checkpoint con tablas Engram (vocabulario comprimido,
primos por *bucket*, multiplicadores por capa) y soso no carga ese modelo. Se deja
registrado qué haría falta si aparece un checkpoint V4: la cadena de
normalizadores del tokenizador (`NFKC → NFD → StripAccents → Lowercase → Replace`)
y las tablas por capa, que son un tipo de tensor más en `sosomodel`. Sirve además
como referencia para un futuro borrador **aprendido** (tabla hash n-grama) que
sustituya al PLD sin tocar la arquitectura del modelo.

---

## 5. Orden, dependencias y esfuerzo

| Orden | Fase | Depende de | Esfuerzo |
|-------|------|-----------|----------|
| 1 | F0 medida | — | 1-2 d |
| 2 | F2 muestreo en GPU | F0 | 1 sem |
| 3 | F1 especulación exacta | F0, F2 | 1-2 sem |
| 4 | F4 snapshot de KV | F0, F1 | 2-3 sem |
| 5 | F3 KV 4 bits | F2, F4 | 1-2 sem |
| 6 | F6 kernels AOT | F0 | 1-2 sem |
| 7 | F7 staging de expertos | F6 | 1 sem |
| 8 | F5 directorio + pull | F4 | 2 sem |

Total: **9-12 semanas** de una persona, con cierres independientes. F2 y F4 son
las que más rinden por semana invertida; F3 sin F4 se queda en ahorro de memoria
sin un consumidor claro.

---

## 6. Qué NO se trae y por qué

| Pieza de DSUF | Motivo |
|---------------|--------|
| Tensor cores FP4 / clusters no portables | Exigen sm_100a/sm_103a; soso tiene GB205/GA107 |
| 1M de contexto en 13 s de prefill | Requiere 4-8 GPU y el KV comprimido del modelo V4 |
| Velo/RDMA y Redis | Sin RDMA en los equipos soportados; el directorio va a sosofs |
| cuDNN/cuBLAS para el codificador de visión | `soso-llm-core` no tiene camino multimodal; entra sólo si algún día se añade visión |
| `tokenizers` (HuggingFace) y `deepseek-recipe` | `soso-llm-core` no puede depender de `std` ni de esas cajas |
| Variantes `variant_*.cu` | Son parches por forma de matriz de un kernel concreto, no diseño |

---

## 7. Riesgos

1. **Exactitud del especulativo (F1).** Un residual mal calculado sesga la
   distribución de forma silenciosa; el chi² es requisito de cierre, no adorno.
2. **KV persistido y obsolescencia (F4).** Un manifiesto sin *fingerprint* fuerte
   produce respuestas incoherentes tras cambiar quant o modelo. Validación
   estricta y recálculo, con test negativo obligatorio.
3. **Divergencia guest/host (F1-F4).** Todo lo que entre en `soso-llm-core` debe
   compilar `--no-default-features`; el snapshot usa el hash ya disponible.
4. **Medir con TCG.** Los escenarios QEMU no sirven para `tok/s`; las cifras de
   velocidad salen del host y de placa, y se anotan con su entorno.
5. **Compilador y ABI CUDA (F6).** Los `.cu` existen; regenerar las variantes
   exige una versión de NVRTC o toolkit que soporte la arquitectura y cabeceras
   compatibles. La validación de símbolos por sí sola no acredita equivalencia.

---

## 8. Fuentes

- Motor y README: <https://github.com/basetenlabs/deepseek-ultrafast>
- Piezas citadas: `crates/engine/src/{decode_cache,kvstore,fp4,weight_staging,engram}.rs`,
  `crates/kv-transfer/src/{snapshot,lib,directory,rdma}.rs`,
  `crates/kernels/{build.rs,src/lib.rs,cu/draft_head8.cu,cu/draft_markov.cu,cu/sample.cu,cu/kv_fp4.cu}`.
- Estado de soso: [`docs/ESTADO.md`](ESTADO.md), [`PLAN-MODELOS-GRANDES.md`](../PLAN-MODELOS-GRANDES.md).

## 9. Revisión de viabilidad y prioridades

La comparación de Baseten corresponde a DeepSeek-V4.1-Flash en 4–8 GPU
Blackwell, con borrador propio; no predice la mejora del PLD de soso. Primero
medir el hardware y la carga reales, incluidos CPU y guest. No se ha ejecutado
un benchmark para este análisis. Los plazos anteriores son orientativos, no
compromisos.

1. **Medición y reutilización local (alta).** F0 debe medir TTFT, prefill y
   decode por separado, latencias p50/p95, memoria, transferencias y aceptación,
   con modelo, cuantización, contexto y backend fijados. Comparar turnos fríos
   y calientes. Antes de diseñar persistencia, medir la reutilización de KV que
   ya existe en el servidor; decidir F4 solo si el prefill repetido pesa.
2. **Especulación (alta, condicionada).** Comparar PLD activado/desactivado y
   verificar si el forward de bloques amortiza su coste. F1 puede empezar en
   CPU sin depender de F2: probar primero distribuciones pequeñas con cálculo
   exacto y después pruebas estadísticas con potencia y semillas predefinidas.
   Un rechazo es un evento interno, no necesariamente un motivo de parada de
   generación. Asegurar rollback de todo el estado, incluidos KV, ventanas y
   cualquier estado recurrente, y probar EOS, cancelación y límites.
3. **Transferencias GPU y expertos (condicional).** Perfilar antes de F2/F7;
   implementar primero el cuello de botella observado. Top-p, penalizaciones,
   restricciones y RNG deben conservar la semántica de `sample.rs`; mantener
   fallback y validar paridad. El backend CUDA de host no acredita GPU nativa
   en soso. El solapamiento de expertos exige medir VRAM y sincronización real.
4. **Snapshots locales (condicional).** Identificar con hash resistente a
   colisiones los pesos y toda la configuración que afecta al estado, incluido
   tokenizador y posiciones; CRC sirve para corrupción accidental, no como
   identidad. Publicar atómicamente, acotar disco y memoria, aislar usuarios y
   probar corrupción, interrupción, cambios de modelo y prefijos divergentes.
   La cola 8192/256 es específica del origen, no una constante portable. Puede
   ser necesario procesar el último token para obtener logits; medir tokens
   realmente reutilizados en vez de exigir cero prefill universalmente.
5. **KV cuantizado (experimental).** F3 solo tras medir presión de memoria y
   pérdida de calidad. Contar escalas y padding: 4 bits más escalas no equivale
   exactamente a un cuarto de f16. Fijar límites numéricos y evaluar calidad en
   contextos largos; coincidencia top-1 absoluta no garantiza calidad ni es un
   umbral realista universal. No presupone tensor cores FP4.
6. **AOT y clúster (baja).** F6 requiere fuentes mantenibles y variantes
   compatibles con cada dispositivo; un cubin de otra arquitectura no es un
   fallback universal. Justificarlo con compilación, arranque y rendimiento
   medidos. F5 requiere primero demostrar que transferir gana a recalcular;
   un fichero local sosofs no es por sí solo un directorio compartido entre
   máquinas: definir propiedad, transporte, coherencia y recuperación ante
   particiones antes de construirlo. Redis y RDMA no son requisitos iniciales.

La licencia ELv2 limita determinados servicios gestionados y exige conservar
avisos; revisar compatibilidad antes de incorporar código o derivados. Evitar
afirmar que una reimplementación concreta queda automáticamente exenta de toda
obligación. El manifiesto upstream usa SHA-256 para ciertas identidades y
BLAKE3 para el contenido; véase [snapshot.rs](https://github.com/basetenlabs/deepseek-ultrafast/blob/main/crates/kv-transfer/src/snapshot.rs).
