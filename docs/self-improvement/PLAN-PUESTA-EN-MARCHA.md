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
82 fichas: **77 hechas, 5 pendientes** (T30, T31, T43, T44, T51: todas de máquina física o de circuito nativo). Validación nativa:
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
| SI-6 OpenCode en soso | **Sustituido por D10**: agente nativo en Rust (T35, hecha); el port de OpenCode no se hace |
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
- **A1 cuantizaciones IQ (L)**, en tres escalones:
  1. *Corrección en CPU (M):* dequantización de IQ4_XS, IQ3_XXS, IQ2_S, IQ2_XS, IQ3_S y de los K-quants que faltan (Q5_K, Q3_K), con las tablas de `ggml-common.h`, validada **bloque a bloque contra
     libggml como oráculo**; dtypes nuevos en `sosomodel`/`gguf2som` (paso directo, sin recuantizar) y matvec escalar correcto.
  2. *Velocidad en CPU (M):* AVX2 para los tipos dominantes (IQ3_XXS, IQ2_S, IQ4_XS).
  3. *GPU (L):* matvec y GEMM de prefill (dequantización en shared) por formato, capas residentes y losas de expertos con los dtypes nuevos (hoy la capa MoE exige gate/up Q4_K y down Q8_0).
- **A2 variante MoE (M):** `qwen35moe` (MoE + Gated DeltaNet + experto compartido) en `gguf2som` y `arch.rs`, CPU primero.
- **A3 kernels GPU (L):** atención lineal (GDN) y experto compartido en CUDA, y flujo de expertos con LRU para el 35B-A3B, reutilizando lo hecho para Mellum y Qwen3-Coder-30B-A3B. Recompilar el PTX tras tocar el `.cu`.
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
