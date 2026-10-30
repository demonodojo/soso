# Automejora de soso: dónde estamos y plan para ponerla en funcionamiento (2026-10-24)

Documento de consolidación. Resume el estado real (con la evidencia de las campañas 1–62 y el banco ciego 5), lo que falta y un plan por
fases con criterio de salida medible. No sustituye al [plan padre](../../SELF_IMPROVEMENT.md), al [índice](README.md) ni a
[DECISIONES.md](DECISIONES.md): los resume y los ordena.

## 0. Qué significa «en funcionamiento»

| Nivel | Descripción | Estado |
|---|---|---|
| **N1 — Host asistido** | El combo local (Qwen3.6) propone parches sobre el repositorio real; un revisor (Claude o el usuario) y una puerta de regresión los filtran; el usuario aplica y commitea. | **Casi**: funciona a mano; faltan generador de tareas, orquestador, regresión y métricas |
| **N2 — Servido por soso** | El mismo bucle con el modelo servido por el runtime de soso (guest o máquina física), no por llama.cpp. | Parcial: runtime con atención lineal en CPU; falta la variante MoE, GPU y medida |
| **N3 — Todo nativo** | Agente (ya nativo), coordinador, build, validación y recuperación dentro de soso, sin Forja ni ejecutores Linux (SI-6/SI-7, T43/T44/T51). | Pendiente |

**Orden acordado (2026-10-24):** primero la **GPU de soso** (opción A y después B, que llevan a N2) con el ciclo N1 de trabajo real en los huecos de GPU; N3 depende de la
máquina física y de cerrar T43/T44/T51.

## 1. Dónde estamos

### Catálogo
92 fichas: **78 hechas, 14 pendientes** (T84–T94 OpenCode/coordinador Sal; T30, T31, T43, T44, T51 máquina física / circuito nativo). Validación nativa:
16 verificadas, 23 parciales, 31 pendientes, 12 no aplicables. Ningún hito SI se declara cerrado en el plan padre.

### Hitos SI, con la evidencia que hay
| Hito | Estado real |
|---|---|
| SI-0 base y evaluación | Hecho en la práctica: base por inventario/hash, bancos R11–R20, R31–R40 y **banco ciego 5 (R41–R50)**, medición con el verificador oficial |
| SI-1 conversación y herramientas | Hecho: render de chat, herramientas del agente nativo, parser tolerante |
| SI-2 servir el modelo desde soso | Hecho con Qwen2.5-Coder-14B y Gemma 4 12B (CPU y GPU en el guest/host). **No** con el modelo actual (Qwen3.6, servido por llama.cpp en el host) |
| SI-3 primera mejora real | Hecha (T22: R03 con 14B a T=0 en host y en guest) y ahora **cuatro parches reales revisados** (R51, R54, R55, R56) |
| SI-4 iteraciones que acumulan | Hecho (T29): ejecutor repetible, 62 campañas; **referencia vigente: banco ciego 5, combo 7/10** |
| SI-5 hardware real | Pendiente (T30, T31): sin máquina física elegida |
| SI-6 OpenCode en soso | **D12** (2026-10-30): reimplementación compatible en Sal (`opencode-sal`) + coordinador Sal; línea T83–T94. T35 (agente Rust) sigue hasta paridad T92 |
| SI-7 compilación/actualización nativas | Parcial: rustc, cargo, C y ensamblador corren en el guest (T40–T42); faltan T43, T44, T51 |

### Capacidad medida del agente local (mismo arnés, 40 turnos, 1 ronda)
| Banco | Gemma 12B | Qwen3.6-35B-A3B (MoE) | Combo MoE → 27B denso | Claude Sonnet 5.5 |
|---|---|---|---|---|
| R31–R40 (ajustado) | 2/10 | 5–7/10 | 9/10 | 10/10 |
| **R41–R50 (ciego)** | — | **3/10** | **7/10** | **10/10** |
| R51–R56 (trabajo real, Clippy) | — | 4/6 | (en curso) | — |

Lectura: el arnés y el banco están bien (Claude lo resuelve todo); **el límite es el modelo local**, y el banco ajustado sobrestimaba. El combo cuesta
≈5 h por 10 tareas difíciles (el denso va a 9–13 tok/s) y ≈25 min por 6 tareas fáciles con el MoE.

### Lo que ya funciona
Agente nativo (`soso-improve-core`, no_std) con puertas antes de terminar (sintaxis, pruebas, Clippy, API pública, ediciones fuera de alcance, poda),
coordinador con estado durable y recuperación, validador independiente, bancos con verificación oficial, lanzadores (`combo-moe-denso.sh`,
`correr.sh`, `claude_proxy.py`), monitores con guardas de memoria y tope de memoria por proceso, procedimiento de revisión de diffs
([revision-trabajo-real](revision-trabajo-real/README.md)).

## 2. Qué falta (por capa)

1. **Bucle de trabajo real (N1).** Hoy todo es pegamento manual. Faltan: generador de tareas desde el repositorio (avisos de Clippy, pruebas rojas, huecos de cobertura),
   modo «mantenimiento» del agente (sin pruebas obligatorias, cambio mínimo), puerta de revisión sistemática (el criterio automático **no basta**: R56 pasaba Clippy
   con una regresión), puerta de regresión (`cargo xtask check` sobre el árbol parcheado), orquestador con cola/presupuesto/guardas, y métricas.
2. **Calidad local.** 7/10 en un banco ciego y 4/6 en tareas fáciles de estilo. Huecos estables: desbordes aritméticos (R42, R50), semántica fina (R49, R37).
3. **Servir desde soso (N2).** El runtime ya tiene Gated DeltaNet en CPU (`forward_gdn_attn`) y `gguf2som` convierte `qwen35` (la arquitectura del 27B denso);
   **no** hay rastro de `qwen35moe` (la variante MoE del 35B-A3B) y no hay kernels GPU para atención lineal. Sin medir: tok/s y memoria en el guest.
4. **Máquina física (N2/N3).** T30/T31: elegir equipo, medir servicio/inferencia y probar la recuperación completa.
5. **Circuito nativo (N3).** T43 (validar y recuperar candidatos construidos en soso), T44 (tres mejoras repetidas) y T51 (campaña completa sin Forja ni Linux).
6. **Higiene.** 42 rutas sin commitear (arnés, scripts de campaña, bancos, documentación); D11 aceptada (2026-10-24); `cargo clippy` falla al compilar en
   `gguf2som`, `soso-llm-api` y `soso-llm-core` (un error, 345 avisos arrastrados); 36 avisos en `soso-improve-core`; `cargo test -p soso-improve-core` necesita `--features std`.

## 3. Plan por fases (orden acordado el 2026-10-24: primero la GPU de soso, A y luego B)

Tamaños: S (horas), M (días), L (semanas). Cada fase termina con un criterio medible.

**Por qué este orden.** El objetivo es que el modelo lo sirva soso, y la GPU es el tramo largo. Las dos «GPU» son incompatibles en la misma máquina a la vez: con el driver NVIDIA
cargado hay CUDA en el host (**opción A**: nuestro runtime con nuestros kernels, L6-H), y con `vfio-pci` persistente la tarjeta pasa a soso pero el host pierde CUDA (**opción B**: GPU
nativa sin Linux, L6 G1→G5). Pasar de una a otra exige reiniciar. Por eso A va primero. Además hay una sola GPU de 12 GB: no se puede desarrollar kernels y correr campañas a la vez, así que
el ciclo de trabajo real (F3) avanza en los **huecos** (tandas nocturnas con llama.cpp, que habla con el mismo API que luego tendrá el servidor de soso).

### F0 — Cerrar y ordenar (S) — en curso
- **Estado (2026-10-24):** arnés v43 marcado como estable ([campana.md](campana.md), «Arnés estable v43»); **D11 aceptada** el 2026-10-24; agrupación de commits propuesta (22 rutas de
  automejora en 3 commits; el resto del árbol —WiFi/iwlwifi, kernel, ROG— es trabajo ajeno); falta revisar R52/R53 de la pasada B de trabajo real.
- **Salida:** árbol limpio y decisiones registradas.

### F1 — Servir Qwen3.6 con el runtime de soso en GPU, en el host (opción A) (L)
Qué tipos de tensor llevan los modelos que usamos (medido en los GGUF): **35B-A3B** (`qwen35moe`) = IQ2_S 52 %, IQ3_XXS 29 %, Q6_K 15 %, IQ4_XS 3 %; **27B denso** (`qwen35`) = IQ3_XXS 59 %,
Q4_K 12 %, Q6_K 10 %, Q5_K 7 %, Q3_K 5 %, IQ2_S, IQ4_XS, IQ3_S, IQ2_XS. Hoy el conversor y el runtime sólo conocen F16/F32/Q4_0/Q4_K/Q6_K/Q8_0/MXFP4. Soportar las IQ permite correr **los mismos pesos
que medimos con llama.cpp** (misma calidad) y que quepan (IQ3_XXS: 27B ≈ 12 GB, 35B-A3B ≈ 13 GB; con Q4_K_M son 16,8 y 22 GB).
- **A0 medir lo que hay (S):** 27B denso `Q4_K_M` (sin formatos nuevos) en el runtime de soso en CPU: ¿carga `qwen35`?, paridad de logits con llama.cpp, tok/s y RAM. Línea base y prueba de la atención lineal.
  **Hecha (2026-10-24, CPU):** `qwen35` carga y converte, pero **no era correcto**: las cabezas q/k de la atención lineal (16) se repiten sobre las 48 de v *en mosaico* (`ggml_repeat`, índice `vh % n_k`), y `forward_gdn_attn` las entrelazaba (`vh / group`). Arreglado; error relativo por capa frente a llama.cpp 27 % → 0,6–1,4 % (ruido de cuantizar activaciones) y logits iguales (Paris 0,444 vs 0,434 con IQ3_XXS). Sondas: `examples/volcar_capas.rs` + `tools/iq-oraculo/volcar_capas.c` (capa a capa) y `logits_ids`. Velocidad sin medir en limpio: con la RAM y el swap al límite da ~10 s/token en CPU y 5,8 s con `SOSO_CUDA=1` (65 matrices residentes, 5,8 GiB).
- **A1 cuantizaciones IQ (L)**, en tres escalones:
  1. *Corrección en CPU (M):* dequantización de IQ4_XS, IQ3_XXS, IQ2_S, IQ2_XS, IQ3_S y de los K-quants que faltan (Q5_K, Q3_K), con las tablas de `ggml-common.h`, validada **bloque a bloque contra
     libggml como oráculo**; dtypes nuevos en `sosomodel`/`gguf2som` (paso directo, sin recuantizar) y matvec escalar correcto.
     **Hecho (2026-10-24):** 7 formatos bit a bit contra libggml (`crates/sosomodel/src/iq.rs`, `tests/iq_oraculo.rs`), dtypes 6–12, `gguf2som`, lector y `matvec_iq`; el 27B UD-IQ3_XXS convierte y reproduce los logits de llama.cpp.
  2. *Velocidad en CPU (M):* AVX2 para los tipos dominantes (IQ3_XXS, IQ2_S, IQ4_XS).
     **Hecho (2026-10-24):** producto entero × Q8_K (`iq_dot.rs`, como `ggml_vec_dot_*_q8_K`) para IQ3_XXS, IQ2_S, IQ4_XS, IQ3_S y Q5_K (la cabeza de logits del 27B IQ3), escalar y AVX2 **bit a bit iguales** (`tests/iq_dot.rs`, con y sin `+avx2,+fma`). Compute puro, 1 hilo, matriz 4096×5120 (`examples/bench_iq`): descuantizar a f32 0,10–0,76 GB/s → Q8_K 5–11 GB/s (×8–50). Logits del 27B IQ3: 174 ms → 36 ms. **Ojo:** el AVX2 es en compilación (`RUSTFLAGS="-C target-feature=+avx2,+fma"`, `CARGO_TARGET_DIR` aparte); sin eso el runtime va escalar, también para Q4_K. La velocidad de extremo a extremo no se puede juzgar aún: el modelo (12 GB) no cabe en la caché de páginas con `/tmp` (tmpfs) ocupado por 8,7 GB de `cursor-sandbox-cache` y el swap lleno, y el decode (~3,5 s/token) es pura E/S.
  3. *GPU (L):* matvec y GEMM de prefill (dequantización en shared) por formato, capas residentes y losas de expertos con los dtypes nuevos (hoy la capa MoE exige gate/up Q4_K y down Q8_0).
- **A2 variante MoE (M):** `qwen35moe` (MoE + Gated DeltaNet + experto compartido) en `gguf2som` y `arch.rs`, CPU primero.
  **Hecha (2026-10-24, CPU):** `gguf2som` convierte `qwen35moe` (capas Gated/GDN alternas, expertos IQ2_S/IQ3_XXS por experto, experto compartido 2D y su puerta `ffn_gate_inp_shexp`); el runtime aplica la puerta sigmoide al compartido. Los expertos/compartidos de dtypes Q4_0, MXFP4 e i-quants ya no se etiquetan F32 por error (`entry_por_dtype`). Paridad con llama.cpp del 35B-A3B UD-IQ3_XXS: 0,160 vs 0,162 (1 token) y 0,411/0,193 vs 0,400/0,198 (5 tokens). Decode en CPU con AVX2, 8 hilos, caché caliente: **~4,2 tok/s** (CUDA actual 4,5: la GPU no ayuda aún con estos dtypes); llama.cpp con GPU da 62–70 → ese hueco es A3.
- **A3 kernels GPU (L):** atención lineal (GDN) y experto compartido en CUDA, y flujo de expertos con LRU para el 35B-A3B, reutilizando lo hecho para Mellum y Qwen3-Coder-30B-A3B. Recompilar el PTX tras tocar el `.cu`.
  **En curso (2026-10-24), medido con el 35B-A3B UD-IQ3_XXS y `SOSO_CUDA=1`:** de ~230 ms/token (4 tok/s) a **~43–50 ms/token en estado estable (~20 tok/s)**, logits idénticos. Lo que lo movió, por orden: (1) `TensorIndex::find` era lineal y un MoE de 256 expertos tiene ~31 000 entradas: 44–68 µs por búsqueda × 27 por capa y token → índice por nombre en `MmapTensorSource`; (2) expertos y compartido agrupados en **una región paralela por fase** (`moe_expertos_agrupados`) y, con GPU, en **un solo lote** (`matvec_lote`: una subida de x, lanzamientos seguidos, una espera); (3) kernel CUDA `iq_matvec` (Q5_K, IQ2_S, IQ3_S, IQ3_XXS, IQ4_XS residentes tal cual; `lxdde/ports/nouveau/matvec_iq.cu`, PTX con **NVRTC** vía `tools/cuda-nvrtc/compilar.py`, sin Docker ni toolkit); (4) Gated DeltaNet: estado recorrido por filas y cabezas repartidas entre hilos (45 → 15 ms de atención/GDN por token). Calentamiento: los primeros ~100 tokens son más lentos (se mapean y verifican los shards de expertos y la VRAM se llena por orden de uso). Pendiente en A3: LRU/expulsión en VRAM (hoy los expertos que no caben van a CPU), kernels de prefill con los dtypes IQ (el prefill sigue en CPU), lote de las matrices de GDN/atención (hoy una espera por matriz) y las vistas de tensores (7–20 ms/token).
- **Prefill y servidor para Qwen3.5/3.6 (2026-10-24, hecho en CPU):** el prefill iba token a token (9 tok/s). Ahora `bloque35.rs` procesa bloques de hasta 256 tokens: proyecciones por lotes, recurrencia GDN y atención token a token, y en el MoE los tokens se agrupan **por experto** (cada experto lee sus pesos una vez y se reparte entre hilos de uno en uno), con producto entero × Q8_K que descodifica cada bloque una vez para todos los tokens (bit a bit igual al de un solo vector). **47 tok/s** en el 35B-A3B en régimen estable (el primer prompt de un proceso tarda ~2× por el CRC y el mmap de los 30 000 shards de expertos); el 27B denso, 6,8 tok/s. Render de chat `qwen35`/`qwen35-pensar` byte a byte con `/apply-template` de llama.cpp (8 casos × 2 modos; `tests/self-improvement/reference/qwen35/`), gramática XML de llamadas reutilizada de `qwen3coder`. **Reutilización de prefijo con capas GDN**: su estado recurrente no se recorta, así que se guardan **instantáneas** del estado al final de cada prompt (las 3 últimas) y se vuelve a la mayor que sea prefijo del prompt nuevo; sin esto cada turno del agente reprocesaba toda la conversación. Comprobado: la respuesta tras reutilizar el prefijo es idéntica a la de un servidor recién arrancado. Servidor con el 35B: `serve_host --profile … ` (perfil `family: qwen35`, stop 248046/248044).
- **A3/A4 en curso (2026-10-25):** hallazgos de la primera campaña con el servidor de soso. (1) Los «error de infraestructura» eran el presupuesto total de 5400 s agotándose con el modelo lento, no un fallo de red. (2) El arnés pliega el historial dentro del primer mensaje de usuario: cada pliegue cambia el prefijo desde ~3,4 k tokens y obliga a reprocesar el contexto entero; con llama.cpp (1000+ tok/s) pasa desapercibido, con 47 tok/s eran 8 min por turno. Remedios: instantáneas GDN cada 2048 tokens, atención AVX2 para `head_dim` 256 (caía al escalar), atención de decode partida por trozos, prefill por bloques también en GPU (gate/up/down de expertos agrupados por experto y matrices densas con `iq_matmul_n`/`mm8n`) y la recurrencia GDN repartida por cabezas sobre el bloque entero. Resultado: prefill de 512 tokens 9,1 s → 4,2 s (122 tok/s), 12 k tokens 305 s → 160 s, decode a 12 k de contexto 95 → 54 ms/token. (3) llama.cpp aplicaba `top_k 20`/`top_p 0,8` por defecto al arnés; el servidor de soso muestreaba a top_p 1,0 y se iba a 8192 tokens. Pendiente: GPU para la atención del prefill y la recurrencia GDN, y repetir el banco ciego 5 entero.
- **Rendimiento del servidor de soso con Qwen3.5/3.6, estado final del tramo (2026-10-26):** 35B-A3B, prefill de 8192 tokens a **~310 tok/s** (de 9 al empezar), decode ~20 tok/s (~13 a 12 k de contexto); 27B denso: prefill ~90 tok/s (de 15), decode ~5. Palancas por orden: prefill por bloques (`bloque35.rs`), producto con tensor cores (`mma.sync` f16→f32) para expertos y matrices densas con decodificadores propios (IQ*, Q5_K, Q4_K, Q8_0, F32; `matvec_iq.cu`), atención del prefill en GPU con KV espejado, Gated DeltaNet entero en GPU (`gdn_capa`), expertos de un bloque en **dos** lanzamientos con combinación en GPU, matrices no residentes subidas «de paso» para el bloque, IQ2_XS (en CPU tardaba 7,5 s por bloque), atención de decode partida por trozos, índice por nombre en `MmapTensorSource`. **Campaña de aceptación con el servidor de soso: 1/10 (llama.cpp 3/10, mismo modelo y banco); ~2× más lenta (4 h).** Siguiente: repetir para medir la varianza y bajar el tiempo (reprocesos tras plegar el historial, expertos no residentes, decode a contexto largo).
- **A4 paridad y rendimiento (M):** misma campaña con el servidor de soso y con llama.cpp (calidad ±1 tarea) y tok/s medidos; el umbral mínimo se fija con esas medidas.
- **Salida:** el combo corre sobre el servidor de soso con CUDA en el host, con los mismos pesos IQ y calidad equivalente.

### F2 — GPU nativa de soso, sin Linux (opción B) (L)
- Requiere ceder la GPU a soso (`l6-g1-vfio-persist.sh --enable` y reinicio; el host pierde CUDA mientras tanto).
- **G4 cómputo** (lanzar kernels vía lxdde + nvkm + SASS) y **G5 inferencia** con los mismos kernels de F1; validar con el chip real (GB205; la 3060/Ampere es el objetivo de validación recomendado).
- **Salida:** la misma campaña con el modelo servido por soso con la GPU controlada por soso, sin Linux ni CUDA.

### F3 — Ciclo N1 de trabajo real (M, en los huecos de GPU)
1. **Generador de tareas** `generar-tareas`: por crate, avisos de Clippy / pruebas rojas → especificación con enunciado, criterio y alcance (prototipo en `campana6`).
2. **Modo mantenimiento:** `--sistema estandar`, enunciado «cambio mínimo que quite el aviso», límite de líneas cambiadas, sin módulos de pruebas obligatorios.
3. **Puerta de revisión:** checklist fija (equivalencia, sin `#[allow]`, sin pruebas no pedidas, sin reestructuración innecesaria) + prueba diferencial para refactors; revisor = Claude
   (cupo de la suscripción: ≈25 min de campaña por ventana) o el usuario.
4. **Puerta de regresión:** `cargo xtask check` y pruebas afectadas sobre el árbol parcheado antes de entregar.
5. **Orquestador** `ciclo`: cola, presupuesto de tiempo/tokens, guardas de RAM/swap/disco/GPU, unidades con `OOMPolicy=continue`, informe y parches revisados en carpeta;
   nunca toca el repositorio real ni commitea.
6. **Métricas:** parches aceptados tras revisión / propuestos, incidentes de infraestructura, tiempo por parche.
- **Salida:** 20 parches reales revisados y aplicados sin regresión en `cargo xtask check`; ≥60 % aceptados tras revisión; 0 incidentes de infraestructura en tandas de 6 tareas.

### F4 — Subir la calidad local (M, en paralelo a F3)
- Tareas de más sustancia que el estilo; probar el denso con razonamiento y otra cuantización; atacar los huecos estables (desbordes, semántica fina), **midiendo en un banco ciego nuevo**.
- **Salida:** banco ciego 6 con el combo ≥ 8/10 sin tocar el arnés durante la medida.

### F5 — Máquina física (L)
- Elegir equipo (docs/HW-MATRIX), medir servicio e inferencia (T30) y probar la recuperación completa de la instalación (T31). **Salida:** 10 tareas consecutivas sin caída ni fuga de recursos (SI-5).

### F6 — Circuito nativo (L)
- T43, T44 y T51: validar y recuperar candidatos construidos en soso; tres mejoras repetidas; campaña completa sin Forja ni ejecutores Linux. **Salida:** SI-7 cerrado con evidencia nativa.

## 4. Decisiones que hacen falta
1. ~~**D11**~~ — **aceptada** (2026-10-24): Qwen3.6-35B-A3B + 27B denso como modelo de trabajo.
2. ~~**Objetivo inmediato**~~ — **resuelto** (2026-10-24): GPU de soso primero (A y luego B), ciclo N1 en los huecos.
3. **Revisor:** Claude con la suscripción (rápido, con cupo limitado) o el usuario; o mixto (Claude propone, el usuario aprueba).
4. **Máquina física** para F4: cuál.
5. **Política de commits:** cómo agrupar lo pendiente y con qué frecuencia entregar parches revisados.

## 5. Riesgos
- **Memoria de la máquina de desarrollo:** Chrome, Cursor y `/tmp` en tmpfs llevaron varias campañas al swap (campañas 52 y 55 inválidas); hay guardas, pero conviene reservar recursos durante las tandas largas.
- **Cupo de la suscripción de Claude:** se agota a los ≈25 min de campaña; no es base de un bucle continuo.
- **Bancos quemados:** cada banco deja de ser ciego tras ajustar el arnés mirando sus fallos; hay que rotar bancos y medir siempre sobre uno nuevo.
- **Criterio automático insuficiente:** sin revisión, un parche con regresión entra (R56).
- **Coste del denso:** ≈30 min por tarea; solo como segunda pasada.
