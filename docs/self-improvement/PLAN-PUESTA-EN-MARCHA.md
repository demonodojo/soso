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

**Recomendación:** apuntar a **N1 primero** (valor inmediato y medible: parches reales revisados) y avanzar N2 en paralelo; N3 depende de
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

## 3. Plan por fases

Tamaños: S (horas), M (días), L (semanas). Cada fase termina con un criterio medible.

### F0 — Cerrar y ordenar (S)
- Commits agrupados (los hace el usuario): arnés (`soso-improve-core`, `tools/soso-improve`), scripts de campaña, bancos `campana5`/`campana6`, documentación y revisiones.
- Aceptar o rechazar **D11**; marcar el arnés v43 como estable.
- **Estado (2026-10-24):** arnés v43 marcado como estable ([campana.md](campana.md), «Arnés estable v43»); agrupación de commits propuesta (22 rutas de automejora en 3 commits; el resto del árbol —WiFi/iwlwifi, kernel, ROG— es trabajo ajeno y va aparte); **D11 aceptada por el usuario el 2026-10-24**.
- **Salida:** árbol limpio y decisiones registradas.

### F1 — Ciclo N1 de trabajo real (M)
1. **Generador de tareas** `generar-tareas`: por crate, avisos de Clippy / pruebas rojas → especificación con enunciado, criterio y alcance (ya hay prototipo en `campana6`).
2. **Modo mantenimiento:** `--sistema estandar`, enunciado «cambio mínimo que quite el aviso», límite de líneas cambiadas, sin módulos de pruebas obligatorios.
3. **Puerta de revisión:** checklist fija (equivalencia, sin `#[allow]`, sin pruebas no pedidas, sin reestructuración innecesaria) + prueba diferencial para refactors; revisor = Claude
   (cupo de la suscripción: ≈25 min de campaña por ventana) o el usuario.
4. **Puerta de regresión:** `cargo xtask check` y pruebas afectadas sobre el árbol parcheado antes de entregar.
5. **Orquestador** `ciclo`: cola, presupuesto de tiempo/tokens, guardas de RAM/swap/disco/GPU, unidades con `OOMPolicy=continue`, informe y parches revisados en carpeta;
   nunca toca el repositorio real ni commitea.
6. **Métricas:** parches aceptados tras revisión / propuestos, incidentes de infraestructura, tiempo por parche.
- **Salida:** 20 parches reales revisados y aplicados sin regresión en `cargo xtask check`; ≥60 % aceptados tras revisión; 0 incidentes de infraestructura en tandas de 6 tareas.

### F2 — Subir la calidad local (M, en paralelo a F1)
- Tareas de más sustancia que el estilo (defectos, huecos de pruebas), con enunciados escritos sin mirar los fallos.
- Probar el denso con razonamiento y otra cuantización; atacar los huecos estables (desbordes, semántica fina) con puertas o prompts, **midiendo en un banco ciego nuevo** (el 5 ya está visto).
- **Salida:** banco ciego 6 con el combo ≥ 8/10 sin tocar el arnés durante la medida.

### F3 — Servir desde soso (L)
- Medir el 27B denso (`qwen35`) en el runtime CPU del guest: tok/s, RAM, calidad frente a llama.cpp (misma campaña, ±1 tarea).
- Añadir `qwen35moe` (MoE + GDN + experto compartido) a `gguf2som` y `arch.rs`, con descarga de expertos a RAM; luego kernels GPU para GDN (como Gemma/Mellum).
- **Salida:** campaña de F1 completa con el servidor de soso y calidad equivalente; umbral de tok/s fijado tras medir.

### F4 — Máquina física (L)
- Elegir equipo (docs/HW-MATRIX), medir servicio e inferencia (T30), probar la recuperación completa de la instalación (T31).
- **Salida:** 10 tareas consecutivas atendidas sin caída ni fuga de recursos (SI-5).

### F5 — Circuito nativo (L)
- T43, T44 y T51: validar y recuperar candidatos construidos en soso; tres mejoras repetidas; campaña completa sin Forja ni ejecutores Linux.
- **Salida:** SI-7 cerrado con evidencia nativa.

## 4. Decisiones que hacen falta
1. ~~**D11**~~ — **aceptada** (2026-10-24): Qwen3.6-35B-A3B + 27B denso como modelo de trabajo.
2. **Objetivo inmediato:** N1 (host asistido) primero, o ir directo a servir desde soso (F3).
3. **Revisor:** Claude con la suscripción (rápido, con cupo limitado) o el usuario; o mixto (Claude propone, el usuario aprueba).
4. **Máquina física** para F4: cuál.
5. **Política de commits:** cómo agrupar lo pendiente y con qué frecuencia entregar parches revisados.

## 5. Riesgos
- **Memoria de la máquina de desarrollo:** Chrome, Cursor y `/tmp` en tmpfs llevaron varias campañas al swap (campañas 52 y 55 inválidas); hay guardas, pero conviene reservar recursos durante las tandas largas.
- **Cupo de la suscripción de Claude:** se agota a los ≈25 min de campaña; no es base de un bucle continuo.
- **Bancos quemados:** cada banco deja de ser ciego tras ajustar el arnés mirando sus fallos; hay que rotar bancos y medir siempre sobre uno nuevo.
- **Criterio automático insuficiente:** sin revisión, un parche con regresión entra (R56).
- **Coste del denso:** ≈30 min por tarea; solo como segunda pasada.
