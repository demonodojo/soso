# Notas de conocimiento verificado (T28)

Una nota es un **resumen con procedencia**, no un registro. Se guarda por generaciones (T46) en `<dir>/conocimiento.<gen>` como una lista JSON.

| Campo | Qué es |
|---|---|
| `id` | 16 hex de `sha256(base, paquete, problema, solución)`: misma nota = mismo id |
| `problema`, `solucion` | texto breve (tope 600 bytes por campo; lo que excede se acota con «(acotado)») |
| `rutas` | archivos de los que habla (tras `::`, símbolo) |
| `pruebas` | comprobaciones que lo respaldan |
| `certeza` | `hecho` (confirmada por el validador) · `hipotesis` (tras un intento, sin confirmar) · `limitacion` (lo que no funcionó o no se pudo probar) |
| `base`, `candidato` | huella de la base y hash del paquete de los que sale |
| `hashes` | sha256 de cada ruta **al escribir la nota** (sirve para caducarla) |
| `evidencias` | `{ruta, sha256}` de lo citado |
| `vigente`, `historial` | vigencia y eventos (propuesta, confirmación, caducidad, con la base de cada uno) |

## Reglas
1. **Hecho sólo con el validador.** `conocimiento confirmar` exige que la referencia de base de la campaña (la escribe `tarea validar`) registre **el mismo paquete** que cita la nota y que cada evidencia exista con su hash. Una limitación no se confirma como hecho.
2. **Caduca.** `conocimiento invalidar --captura <base nueva>` marca `vigente: false` las notas cuyos archivos tienen otro hash (o ya no están) y anota el motivo en `historial`. No se borra nada.
3. **Recuperar** (`conocimiento recuperar --estado … --run …`): sólo hechos y limitaciones **vigentes**, puntuados por solapamiento de rutas editables de la tarea (×10) y palabras del problema; empate por `id`; corte por presupuesto de bytes (lo que no cabe se cuenta). Las hipótesis y lo caducado no llegan al agente.
4. **No viaja:** notas que hablan de rutas reservadas (`--reservado-prefijos`), de archivos de credenciales (`.env*`, `*.pem`, `*.key`, `id_rsa`…) o cuyo texto huele a secreto (`Bearer`, `api_key`, `BEGIN PRIVATE KEY`, `password`, `token=`…). Se listan como excluidas, con el motivo. Los campos acotados impiden colar una transcripción.
