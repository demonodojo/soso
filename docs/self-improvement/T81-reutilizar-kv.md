# T81 — Reutilizar el prefijo del KV entre peticiones

**Origen:** destapado el 2026-10-08 al intentar [T22](T22-primera-mejora.md). **Estado:** hecha (2026-10-08).

## Reproducción

El prefill del 7B en el guest va a ~1 s por token (0,9–1,1; sin mejora con 4 núcleos). OpenCode manda ~5 000 tokens en cada petición (el system prompt y 16 KB de esquemas de herramientas son idénticos en todos los turnos), y el servidor reiniciaba la caché K/V en cada petición. Primera petición de T22: **1 187 s sin respuesta** (plazo agotado); un turno costaba más de una hora y una tarea de seis turnos, más de seis.

`target/self-improvement/first-improvement/medida-prefill-smp4.txt`: 137 tokens → 146,6 s; 443 tokens → 400,9 s.

## Cambio

- `LayerKv::truncate_tokens` y, en `Runtime`, `begin_sequence_reusing(prompt)`: calcula el prefijo común entre el prompt nuevo y los tokens que ya tienen K/V (`cached_tokens`), recorta la caché a ese prefijo y sólo procesa el resto. Siempre se procesa al menos el último token (hacen falta los logits).
- **Exacto**, no aproximado: por causalidad el K/V de los primeros `p` tokens depende sólo de ellos, así que el estado es el de un prefill limpio de `prompt[..p]`. Pruebas en `crates/soso-llm-core/tests/generate.rs`: la salida con prefijo reutilizado es idéntica a la de un prefill en frío (ruta normal y ruta con planificador).
- Sólo para atención clásica: no se aplica a MLA ni GDN (estado no recortable), a un MoE con planificador, ni si el K/V ha sufrido expulsión (`tokens != pos`). Una generación cancelada o fallida deja la caché sin anotar y la siguiente empieza de cero.
- `generate_stream_par_observed` y `generate_stream_planned_observed` (la que usa el servidor del guest) reutilizan; el servidor del guest y el backend host dejan de reiniciar la caché por petición (`cerrar_peticion`, `CpuBackend::begin_request`).

## Medida (guest, 7B, `-smp 4`)

| Petición | Tokens | Tiempo |
|---|---|---|
| A (frío) | 545 | 518 s |
| B (mismo prefijo + ~11 nuevos) | 556 | **19,9 s** |
| A otra vez | 545 | **10,0 s** |

`target/self-improvement/first-improvement/medida-reuso.txt`.

## Límites

- El primer turno de cada sesión sigue pagando el prefill completo (~5 000 tokens ≈ 80 min con el prompt actual de OpenCode). Reducirlo exige menos herramientas o un prefill por lotes (GEMM), que no está.
- `-smp 8` hace que `soso-llm serve` termine al arrancar (sin salida); con 4 funciona. No investigado.
