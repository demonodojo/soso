---
name: soso-self-improvement
description: >-
  Automejora de soso con OpenCode y el modelo local: fichas T01–T75, hitos
  SI-0–SI-7, catálogo tasks.json, contratos C1–C7/NATIVO, soso-improve,
  soso-llm-core conversation/chat, soso-llm-api, banco de casos y
  native_validation. Úsala al oír automejora, self-improvement,
  SELF_IMPROVEMENT, OpenCode, soso-coder, soso-improve, Txx, SI-*,
  «avanza / continúa / siguiente ficha», render de chat, herramientas del
  modelo o coordinador. No uses soso-architecture ni el plan padre entero
  para esta ruta: esta skill basta para elegir e implementar una ficha.
---

# soso — Automejora

Objetivo: un agente OpenCode usa el modelo servido por soso para mejorar soso.
Destino: todo el circuito **dentro** de soso. Linux/Forja son etapas transitorias.

Esta skill es la entrada. No leas `soso-architecture`, `SELF_IMPROVEMENT.md`
ni todas las fichas. El índice operativo es
[`docs/self-improvement/README.md`](../../../docs/self-improvement/README.md).

## Arranque (este orden, nada más)

1. Esta skill (ya).
2. **Sólo** «Por dónde empezar» de
   [`docs/self-improvement/README.md`](../../../docs/self-improvement/README.md)
   (hasta el prompt listo para copiar). Ahí está la ficha recomendada.
3. Elegir **una** Txx:
   - el usuario nombra `Txx` → esa;
   - «avanza / continúa / siguiente» → la recomendada del README;
   - reanudar → `tracking.next_step` de la ficha `in_progress`, no T01.
4. En [`tasks.json`](../../../docs/self-improvement/tasks.json), recortar **ese**
   id (`rg '"id": "T06"' -A 45 docs/self-improvement/tasks.json`). Leer
   `status`, `depends_on`, `entry_conditions`, `contract_sections`, `context`,
   `tracking`, `native_validation`. No cargues el JSON entero.
5. Dependencias `done` **y** condiciones de entrada con evidencia vigente.
   Si falta algo: registrar bloqueo; otra ficha independiente solo si el
   encargo lo permite.
6. Leer la ficha `docs/self-improvement/Txx-….md`,
   [`NATIVO.md`](../../../docs/self-improvement/NATIVO.md) si implementas, y de
   [`CONTRATO.md`](../../../docs/self-improvement/CONTRATO.md) **sólo** las
   secciones `C…` de la ficha.
7. `seguimiento/Txx.md` si existe (reanudar). Si no, créalo al empezar.
8. Código: archivos de `context` y los que la ficha **permite cambiar**.
   Símbolos que falten; no el módulo entero «por si acaso».
9. Una ficha por sesión. El plan completo = esa ficha y **parar**; no encadenar
   T07 al cerrar T06. La ficha lo dice.

Un diagnóstico u otro archivo abierto en el IDE no cambia el objetivo.

## Qué no cargar

| Tentación | Por qué no |
|---|---|
| `SELF_IMPROVEMENT.md` entero | Padre de 450 líneas. Hitos SI-* ya están en el README. |
| El catálogo / `tasks.json` completo | Una ficha. El índice nombra la siguiente. |
| `CONTRATO.md` entero | Solo `contract_sections`. |
| `soso-architecture` / `soso-dev` | Kernel/QEMU genéricos. Dev solo si la ficha pide `xtask test/check`. |
| `chat.rs` / runtime / tokenizer enteros | Salvo que `context` o «archivos que se pueden cambiar» los listen. |
| Pesos GGUF | Las pruebas de tokenizer/render usan `tokenizer.som`, no los 2,2 GB. |

## Elegir ficha

Candidata = `pending` (o `in_progress`), `depends_on` todas `done`,
`entry_conditions` cumplidas. `done` de un informe no-go **no** habilita al
consumidor que exige `go` (T14 → T16/T22). `native_validation.pending` **no**
bloquea implementar: bloquea declarar ejecución guest.

Frentes independientes actuales: T74 (evaluación) y T72/T75 (bootstrap).
Comprobar siempre el README y el catálogo; no reutilizar listas históricas.

Nombres de módulos/comandos en fichas pendientes son **entregables**, no APIs
existentes. Crearlos en esa ficha.

## Estados

| `status` | Significa |
|---|---|
| `pending` | No iniciado |
| `in_progress` | Empezado; cierre incompleto |
| `blocked` | Falta una dependencia/entrada concreta |
| `done` | Cierre de la ficha + evidencia. No implica guest. |

`native_validation.status`: `pending` / `partial` / `verified`.
`not_applicable` solo laboratorio con consumidor nativo. **No** pasar a
`verified` por `done`, Rust o un ELF cruzado.

Al empezar trabajo real: `in_progress`. Test fallido que se corrige sigue
`in_progress`. Pausar sesión no es `blocked`.

## Cierre (los cuatro a la vez)

Convención completa:
[planes.md](../soso-architecture/references/planes.md)
(«Persistencia y sincronización»). Mínimo en la misma entrega:

1. `tasks.json`: `status`, `tracking` (`updated_at` ISO, `base` = commit + hash
   del diff sucio, `summary`, `evidence`, `blocker`, `next_step`).
2. Ficha: encabezado, casillas acreditadas, fecha, enlace al resumen.
3. README: fila del índice y «Por dónde empezar» si cambia la recomendada.
4. `docs/self-improvement/seguimiento/Txx.md`: crear al empezar; entradas por
   intento (fecha/base, hecho, pruebas, límites, bloqueo, próximo paso).

Logs: host `target/self-improvement/tasks/Txx/`; guest
`/var/self-improvement/tasks/Txx/`. El resumen git debe conservar comandos,
exit codes y conclusiones aunque se limpie `target/`. No inventar artefactos
en `tracking.evidence`.

`SELF_IMPROVEMENT.md` / hitos SI-* solo cuando **cambia** el hito padre.
Cerrar SI-0 no es contar T01–T03: falta T14.

Esta skill: actualizar «Por dónde» de abajo y comandos si la ficha los cambia.
`soso-dev` si aparece un `xtask` nuevo. Manual solo con UX de usuario real.

## Implementación

- Lógica en Rust `no_std + alloc`. Adaptadores: `tools/soso-improve` (host) y
  `user/soso-improve` (guest). No Python/Bash/Git CLI como requisito permanente.
- Reutilizar crates; no recrear `soso-improve-core`.
- `chat::render` (pregunta única) se mantiene. Conversación nueva:
  `conversation::render_messages` y amigos.
- JSON en plantillas: `serde_json`, no comillas a mano.
- Tests de modelo real: tokenizer/fixtures de T03, no un mock. Sin
  `tokenizer.som` el test falla con ruta clara; no se salta en silencio.
- Alcance: lo que la ficha permite. Fuera → ficha nueva con reproducción.

## Ejecuciones largas: vigilarlas y reaccionar sin que nadie te despierte

Cuando una prueba, un build en el guest o una suite dura más de unos minutos, **la espera es
parte del trabajo**. Costó horas en T42 (2026-10-06/07): una etapa del cargador se colgó o
falló a los 5 min y nadie lo miró hasta el siguiente aviso del usuario, y una cadena de pasos
siguió ejecutándose sobre un fallo anterior sin sentido.

1. **Todo lo que lances en segundo plano lleva un vigilante que acaba en los tres casos**:
   termina bien, falla (`error`, `abort()`, `Caused by`, `FALLA`, exit ≠ 0) o **se queda
   parado** (el log no crece, o el proceso hijo ya no existe). Con `Monitor`, o un `until` en
   segundo plano que acote el tiempo (`timeout` ≈ duración esperada × 2). Un `until [ -f listo ]`
   solo avisa del caso feliz y se queda mudo ante un cuelgue.
2. **Al llegar el aviso (o en cada vistazo), lo primero es leer la salida y actuar**: si falló,
   diagnosticar, arreglar y relanzar en ese mismo turno; si acabó, pasar al siguiente paso.
   No escribas «espero» y pares si ya hay algo que leer. Si el aviso no vino, **mira tú**
   cada 5–10 min (`ScheduleWakeup`/`Monitor` con latido) en vez de esperar.
3. **Detecta cuelgues activamente** en el guest: `ssh … ps` y comprueba que haya un proceso
   hijo (`rustc`, `clang`…) vivo. Un `cargo` en `futex` **sin hijos** durante varios minutos
   es un cuelgue: mata el QEMU (sólo el tuyo) y reintenta; no esperes una hora.
4. **Una cadena de pasos se corta en el primer fallo** (`set -e`, o comprobar el código y
   `exit`), y escribe un fichero `fallo-<paso>` que el vigilante vea. Nada de seguir con el
   paso 4 tras fallar el 3.
5. **Mientras corre algo largo, avanza lo independiente** (documentar, preparar el siguiente
   fixture, leer el código del siguiente hueco). Ningún bloque de espera sin trabajo útil a la
   vista.
6. **Anota la duración esperada** de cada paso largo en el seguimiento (p. ej. «etapa 2:
   14 min, etapa 4: 4 min, `core` de 32 bits: 20 min») para reconocer un cuelgue por pasarse el
   doble.
7. Si el usuario dijo «continúa sin parar», el siguiente paso se lanza **en cuanto** acaba el
   anterior, y las preguntas sólo para lo que de verdad no se puede decidir sola.

## Mapa corto

| Pieza | Dónde |
|---|---|
| Plan padre | `SELF_IMPROVEMENT.md` (no leer al implementar) |
| Índice / siguiente ficha | `docs/self-improvement/README.md` |
| Catálogo | `docs/self-improvement/tasks.json` |
| Contratos | `CONTRATO.md` (C1–C7), `NATIVO.md` |
| Perfil del modelo | `docs/self-improvement/modelo.md` |
| Fixtures chat | `tests/self-improvement/reference/` |
| Tokenizer/pesos convertidos | `target/qwen2.5-coder-3b-model/` (`tokenizer.som` v2) |
| Original HF (T03) | `target/self-improvement/modelo/` |
| Chat dominio | `crates/soso-llm-core/src/conversation.rs` |
| Coordinador | `crates/soso-improve-core`, `tools/soso-improve`, `user/soso-improve` |

Perfil de trabajo: **Qwen2.5-Coder-14B-Instruct** (`qwen2.5-coder-14b`, id `soso-coder`, D9); el **7B** es el perfil ligero (T20/T21). El 3B es el candidato anterior.
T52/T53: segmentación BPE **igual** a la referencia (5/5). T06 no puede
debilitar eso a una comparación aproximada.

## Comandos

C-144 (jobserver): las fuentes corregidas están en
`config/rust-soso/vendor/jobserver` (0.1.34). `apply-patches.sh` las copia y
configura la resolución local en Rust y Cargo; no parchear la caché de
jobserver. Regresiones host sin Python ni registry:
`tests/self-improvement/native/cargo/jobserver/probar-host.sh`.

Los de **la ficha**, después de crearlos. Típicos:

```sh
# Ficha T06 (ejemplo): lo que escriba su sección Comprobación
cargo test -p soso-llm-core --features std --test conversation_render
cargo check -p soso-llm-core --no-default-features

# Guest (cwd user/): C1 — std no se cuela
cargo build --release -p soso-llm   # desde user/

# Coordinador T01/T02
cargo test -p soso-improve-core -p soso-improve
cargo run -q -p soso-improve -- modelo comparar --modelo target/qwen2.5-coder-3b-model

# Captura de base (no toca el checkout)
cargo run -q -p soso-improve -- capturar --repo . --out target/self-improvement/base
```

`cargo xtask check` / `test` al **promover una base**, no por cada ficha de
tipos. QEMU no acredita una entrega solo de seguimiento o de `no_std` host.

## Campañas con modelos locales (lecciones de las campañas 22–29)

Detalle en [campana.md](../../../docs/self-improvement/campana.md) («Campañas 22–29»). Lo que ahorra tiempo:

- **Ante cero aceptadas, `LimitePasos` o errores raros, abre la conversación de la primera tarea** (`ejecucion-N/R31/estado/*.conversacion.json`) **antes de culpar al modelo.** Cinco de las seis tandas «malas» eran defectos del agente: tope de 64 mensajes del servidor (`MAX_MENSAJES`; `Config::mensajes_max = 60` pliega y deja un historial de una línea por llamada), ediciones sin efecto que devolvían «Hecho», guardas de repetición que no veían fallos alternados.
- **`--max-turnos` se recortaba a 40 en silencio hasta el binario v12** (`runner.rs`, `Limites`): con binarios anteriores `MAX_TURNOS=60/150` no valía; compruébalo si ves `LimitePasos` con ~38 herramientas.
- **`--max-turnos` no se leía en el modo `tarea` hasta el binario v18**: las campañas anteriores corrieron con 40 turnos aunque se pidieran más; comprueba `"pasos"` en `estado/*eventos.json`.
- **Equipo planificador+programador+revisor** (`--multiagente`): con Gemma 12B el planificador no da un plan limpio y cada tarea tarda más de una hora; apagado.
- **Revisión de lo hecho** (`revisar`, `--revisar-final`, `--revisor`; `agente/revision.rs`): con Gemma 12B no sube el pass@1 (autorrevisión 1/10, revisor independiente 2/10, sin revisión 3/10). El agente se autoconfirma y, con revisor aparte, parchea mal y agota turnos. Apagadas por defecto.
- **El prompt de «pruebas propias» (`SISTEMA_AGENTE=pruebas`) empeora a Gemma** (1/10 frente a 3/10). No es el valor por defecto.
- **Mejorar las herramientas no movió el pass@1** (ruido ±1–2 tareas de 10): no seguir afinándolas sin banco nuevo. El banco R31–R40 ya no es ciego.
- **No cambies el binario del agente con la campaña en marcha** (cada tarea lanza el binario de ese momento) ni lances dos campañas a la vez (GPU y puertos compartidos); renombra con sufijo las carpetas de tandas descartadas en vez de borrarlas.
- **Arranque (GPU del host):** servidor `systemd-run --user --unit=serve-gemma --setenv=SOSO_CUDA=1 --setenv=HILOS=16 --setenv=SOSO_LLM_API_KEY=clave-host target/mellum-work/serve_host-cuda --model-dir target/gemma4-12b-model --profile target/gemma-work/perfil.json --port 7442`; campaña con `correr.sh` y `ROUNDS`, `TEMP_REINTENTO=0.7`, `MAX_TURNOS=60`, `SOSO_IMPROVE_BIN`, `CAMPANA_JSON=tests/self-improvement/campaign-12.json`. Vigilante con parada si hay < 1,5 GB de RAM disponibles, < 3 GB de disco, 90 min sin progreso o el servidor cae.

## Tras cada campaña: analizar los rechazos (obligatorio)

Antes de lanzar la siguiente: `python3 tests/self-improvement/campana/analizar.py DIR_CAMPAÑA --bin target/gemma-work/target-vN/debug/soso-improve` genera `ANALISIS.md` con, por tarea no aceptada, el diff de lo cambiado, las pruebas reservadas que fallan y su mensaje, y las estadísticas de ediciones. Lee cada diff contra «Qué se pide», clasifica la causa (requisito no atendido o a medias, no compila, borrado de código ajeno, lógica mal ubicada, no implementó nada) e **implementa sin pedir confirmación** la mejora genérica que la evitaría (instrucción permanente del usuario); cuenta qué implementaste y qué descartaste y por qué. Guarda el resumen en `docs/self-improvement/resultados-campanas/campana-N/analisis.md`. Cargo recompila por la FECHA de los fuentes, no por su contenido: el script actualiza la fecha de lo editado y usa su propio `CARGO_TARGET_DIR`; no lo cambies a mano o saldrán pruebas fallando de más.

## Entrega al siguiente agente

Plan/ID, estado (catálogo = ficha = fila), evidencia, bloqueo, siguiente ficha
**habilitada**. Distinguir host `done` de guest `native_validation`.

## Por dónde (actualizar al cerrar la recomendada)

Estado vigente y siguiente ficha en [README](../../../docs/self-improvement/README.md).

**2026-10-30 (D12 / OpenCode en Sal).** T83–T94 **hechas**: D12, `sal/std/soso`, repo `opencode-sal`, `cargo xtask pkg-pack opencode-sal`, guest `opencode --version`. Coordinador: `soso-improve-sal campana`. Paridad/autonomía: [tests/self-improvement/opencode-sal/README.md](../../../tests/self-improvement/opencode-sal/README.md).

**Actualización 2026-10-24 (línea T29 / campañas).** Cifra de referencia: **banco ciego 5 (R41–R50): combo Qwen3.6-35B-A3B → Qwen3.6-27B
denso 7/10** (MoE solo 3/10; Claude Sonnet 5.5: 10/10 como techo); el banco ajustado R31–R40 daba 9/10 (inflado). Gemma 12B: 2/10. Modelo de trabajo D11 **aceptada** (2026-10-24). Antes de lanzar
una campaña: [campana.md](../../../docs/self-improvement/campana.md) y [seguimiento/T29.md](../../../docs/self-improvement/seguimiento/T29.md).
Lanzadores en `tests/self-improvement/campana/`: `combo-moe-denso.sh NOMBRE` (MoE + denso), `qwen36-servir.sh`, `qwen27-servir.sh`,
`denso-solo.sh`, `claude-piloto.sh`/`claude_proxy.py` (Claude por la suscripción; se niegan a arrancar con `ANTHROPIC_API_KEY`).
Trampas operativas: el servidor Qwen necesita ≈10 GB de RAM libres y Chrome/Cursor/`/tmp` (tmpfs) la agotan (campañas inválidas por
tiempo agotado: vigila la memoria y el swap); `ls` es eza en este shell (`/bin/ls` en scripts); el validador sale «Bloqueado» («la base cambió») si
reutilizas una carpeta de salida tras una ejecución abortada (`correr.sh` ya borra `refs/base-campana-Rxx.*` al empezar; en campañas viejas
repite a mano con `soso-improve verificar repo --caso Rxx --banco campanaN --arbol RUTA/copia`); la inyección de
caída a los 30 s pilla a los modelos rápidos tras su primera edición (usa `campaign-15.json`, sin inyección); `campaign-16.json` da 5400 s
por tarea al denso. El banco R31–R40 ya no es ciego: lo siguiente es un banco nuevo.
**10-oct-2026:** T22 está **hecha** con Qwen2.5-Coder-14B a T=0 (OpenCode + herramienta propia
`reemplazar_texto`, verificador exit 0) en el host **y en el guest** (14 GiB); el perfil T14 sigue siendo el 7B (a T=0 no
resuelve R03). OpenCode sólo envía `temperature` si el modelo declara `"temperature": true` en `opencode.json`.
**Modelo de trabajo de T25–T29: `qwen2.5-coder-14b`** (D9, aceptada 11-oct; GO 10/10 en T14, lock `model-lock-14b.json`); el 7B es el perfil ligero. Siguiente: T35.

**15-oct-2026 (campañas con otros modelos):** Gemma 4 12B servido por soso dio 5/10 y 5/10 en R11–R30 (GO mínimo de SI-4) y 2/10 en el banco ciego R31–R40 (3/10 pensando); **Mellum 2.1 12B dio 1/10, 1/10 y 0/10 en R31–R40** (peor que Gemma; tres tareas acaban con «el candidato no cambió nada»). Resúmenes en `docs/self-improvement/resultados-campanas/` y `campana.md`. Operativa que ya no hay que redescubrir:
- Arnés: `tests/self-improvement/campana/correr.sh DIR [tareas]` con `CAMPANA_JSON`, `MODELO`, `PUERTO`, `MAX_TOKENS`, `BASE_INICIAL`, `CARGO_TARGET_DIR`, `SOSO_IMPROVE_BIN`; lanza la tarea siguiente ~1 s después de la anterior (**nunca cambies el servidor a mitad**). Lo largo, con `systemd-run --user` y un vigilante (éxito, error, servidor caído, estancamiento).
- Servidor en GPU del host: `SOSO_CUDA=1 serve_host` (ver skill `soso-dev`, «Modelos en la GPU del host»); un solo hueco, así que un 429 es que sigue ocupado con una petición abandonada.
- **El estimador de contexto del agente depende del tokenizador:** `Config::chars_por_token` (3,0 por defecto; el adaptador de host lo lee de `SOSO_AGENTE_CHARS_POR_TOKEN`). Con Mellum son ~1,5 car./token y a 3,0 el recorte nunca actuaba (422 en R35/R36).
- **Reintentos:** `ROUNDS=3` en `correr.sh` (rondas 2–3 con nota genérica de rechazo y `TEMP_REINTENTO`=0,7). A T=0 reintentar repite la ronda 1; con temperatura aporta +1…+4 tareas por tanda (Gemma, campaña 12: 1–3/10 a la primera, 3–5/10 con 3 rondas). Útil como generador de candidatos con validación independiente; no es comparable con el mínimo de SI-4 a un intento.
- **Agente v3:** aviso «no has modificado nada» (`avisos_sin_editar`) y guardián de llamadas repetidas que ya salieron bien desde la última edición (`repetidas_ok_max`, cuenta aunque se alterne): evita el bucle de `cargo test` en verde que agotaba las 30 herramientas.
- Una tarea que agota la hora con pocas llamadas no es del modelo: mide primero tok/s de prefill y decode del servidor.

Revisión del **2-oct-2026**: el perfil elegido es `qwen2.5-coder-7b` (campaña 10×3, cobertura `completa`, **GO 10/10**; el 3B quedó NO-GO 8/10). **T20** está pendiente con esa identidad. **T40 y T41 están cerradas** (2026-10-02: `cargo build --offline` compila en el guest un workspace de dos crates, uno con `build.rs`, y `soso-abi`). **T40, T41 y T80 están cerradas; T42 en curso** (2026-10-03: `mkfs-soso` compila en el guest —2 min 28 s— y su imagen se verifica con el `sosofs` del host; faltan C-113 `-Zbuild-std`, C-115…C-117 C/ensamblador/objcopy y la imagen arrancable). Truco: `SOSO_REUSE_DATA=1 cargo xtask run` conserva lo que el guest escribió.

Trabajo independiente: **T72** está cerrada (26-sep): `soso-alloc` lo usan
libsoso y soso-rt; la sonda guest `monton` sale 7/7. **T75** está cerrada
(26-sep): la receta iguala al script en copias de 32d94cc9 y la sonda guest
`receta/espejo-resto-ajeno` pasó junto al humo de directorio vacío (exit 0).
**T76** (tokenizer en tests) está cerrada. T39 sigue bloqueada por el build
de std: los dos errores del alocador (`cfg_select` / `imp`) ya no están; el
intento del 26-sep (rc=1) falla por `implicit_provenance_casts` en
`sys/random/soso.rs` y `sys/sync/futex/soso.rs`, y por avisos en deny.
T73 está cerrada. T08/T18/T45/T46 están terminadas: no volver a recomendarlas.

T20 está pendiente: el perfil es el 7B. T21/T22 y T25–T29 esperan esa cadena.
T16/T17 tienen evidencia guest parcial, no validación completa. El port de
OpenCode sigue pendiente; N-xxx decidida no significa implementada.
[DECISIONES.md](../../../docs/self-improvement/DECISIONES.md) separa ruta elegida
y trabajo aplazado. Revisar antes de proponer otra decisión ya tomada.

Al **revisar/replanificar** por petición expresa, inspeccionar resúmenes del
catálogo, seguimientos recientes y evidencia de los frentes afectados; la
regla de una ficha limita implementación, no impide auditar el plan. No
iniciar implementación ni campañas largas por una actualización documental.
