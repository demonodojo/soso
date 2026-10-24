# Revisión de los parches del agente local sobre trabajo real (2026-10-24)

Primera tanda de «trabajo real»: seis tareas de limpieza de Clippy sobre una instantánea del último commit (`7a8db4a26`, banco
[campana6](../../../tests/self-improvement/campana6/)), resueltas por el combo Qwen3.6-35B-A3B → 27B denso (arnés v43). Criterio objetivo del
validador: `cargo clippy -D warnings` sin avisos y pruebas del crate en verde. **Revisión: Claude, diff a diff**, además de repetir el criterio y,
cuando hubo dudas, una **prueba diferencial** (misma entrada contra el código original y contra el parcheado).

Los parches de [`parches/`](parches/) son los **ya revisados** (no los brutos del agente). Se aplican desde la raíz del repositorio con
`git apply docs/self-improvement/revision-trabajo-real/parches/Rxx.patch`; comprobado con `git apply --check` sobre el repositorio real (no se
han aplicado ni commiteado: eso lo decide el usuario).

| Tarea | Agente | Veredicto | Qué se cambió en la revisión |
|---|---|---|---|
| R51 `block-dev` | MoE, aceptada (149 s) | **Aceptado** (código) | Quitado un módulo `pruebas_agente` de 19 pruebas (≈180 líneas) que nadie pidió; varias triviales (p. ej. «`MemBlockDevice::new` crea ceros»). El cambio de código —`is_multiple_of` ×2 y `as_chunks_mut()` en lugar de `chunks_exact_mut` + `try_into`— es equivalente: la guarda previa ya exige un múltiplo de `BLOCK_SIZE`. |
| R54 `soso-log-core` / `soso-red-core` | MoE, aceptada (277 s) | **Aceptado** (código) | Quitados dos módulos de pruebas no pedidos, colocados además a mitad del archivo. `write!(..\n)` → `writeln!` en cuatro sitios: mismo texto de salida. |
| R55 `soso-update-core` | MoE, aceptada (112 s) | **Aceptado** (código) | Quitado el módulo de pruebas. Un solo cambio: `!referencias.iter().any(\|r\| *r == id)` → `!referencias.contains(&id)`. No toca `pack.rs` (el aviso `items_after_test_module` no salta en el build normal). |
| R56 `soso-web-core` | MoE, aceptada (206 s) | **Aceptado con corrección** | **`decode_entities` cambiaba el comportamiento:** con un punto de código inválido (`&#xD800;x`, `&#1114112;y`) el original descarta la entidad (`"x"`, `"y"`) y el código del agente la dejaba literal (`"&#xD800;x"`). Lo ve la prueba diferencial, no el criterio. Sustituida por la versión mínima fiel (`if … && let Some(..)`). `bytes_to_text` y la reestructuración de `html.rs` del agente son equivalentes (24 entradas idénticas), aunque `html.rs` queda más largo de lo necesario. |
| R52 `gptdisk`, R53 `soso-http` | MoE rechazadas; denso en curso | pendiente | — |

## Lo que enseña la revisión
- **El criterio objetivo no basta para limpiezas «sin cambio de comportamiento»**: R56 pasaba Clippy y las pruebas con una regresión. Hace falta revisar el diff (o una prueba diferencial) en toda refactorización.
- **Las pruebas propias obligatorias añaden ruido en tareas de mantenimiento** (R51, R54, R55): para este tipo de trabajo conviene `--sistema estandar` (sin pruebas obligatorias) o descartar los módulos al revisar.
- El agente tiende a **reestructurar más de lo que Clippy pide** (R56): conviene pedir en el enunciado «el cambio más pequeño que quite el aviso».
