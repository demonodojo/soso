---
name: soso-self-improvement
description: >-
  Automejora de soso con OpenCode y el modelo local: fichas T01–T53, hitos
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
| Las 53 fichas / `tasks.json` completo | Una ficha. El índice nombra la siguiente. |
| `CONTRATO.md` entero | Solo `contract_sections`. |
| `soso-architecture` / `soso-dev` | Kernel/QEMU genéricos. Dev solo si la ficha pide `xtask test/check`. |
| `chat.rs` / runtime / tokenizer enteros | Salvo que `context` o «archivos que se pueden cambiar» los listen. |
| Pesos GGUF | Las pruebas de tokenizer/render usan `tokenizer.som`, no los 2,2 GB. |

## Elegir ficha

Candidata = `pending` (o `in_progress`), `depends_on` todas `done`,
`entry_conditions` cumplidas. `done` de un informe no-go **no** habilita al
consumidor que exige `go` (T14 → T16/T22). `native_validation.pending` **no**
bloquea implementar: bloquea declarar ejecución guest.

Independientes típicas (comprobar el README, no esta lista): camino SI-1
(T06/T07), T08, T18, T45, T46, inventarios T32/T36/T38.

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

Familia fijada: **Qwen2.5-Coder-3B-Instruct** (`qwen2`, id `soso-coder`).
T52/T53: segmentación BPE **igual** a la referencia (5/5). T06 no puede
debilitar eso a una comparación aproximada.

## Comandos

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

## Entrega al siguiente agente

Plan/ID, estado (catálogo = ficha = fila), evidencia, bloqueo, siguiente ficha
**habilitada**. Distinguir host `done` de guest `native_validation`.

## Por dónde (actualizar al cerrar la recomendada)

Camino SI-2: **T10** (API JSON; deps T05–T06–T08 listas). T09, T18, T45 y T46
también habilitadas. T06–T08 cerradas. Validación nativa pendiente de T45/T49.
