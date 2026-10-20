**Resultado:** 3 aceptadas de 10 (R32, R36, R40), una ronda, 0 errores de entorno ni `LimitePasos`.

Gemma 4 12B por soso (GPU del host), agente v10, **prompt estándar**, banco R31–R40 con `campaign-12.json` (60 herramientas y **40 turnos efectivos** (el arnés recortaba `--max-turnos` a 40 hasta el binario v12)). Referencia en una ronda sobre este banco: agente v2 2/10 (campaña 9), v3 1–3/10 (campaña 12). Dentro del ruido: no demuestra mejora de pass@1; sí que el agente ya no falsea el resultado.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 1 | rechazada | programacion | 17 | 109 |  |
| R32 | 1 | aceptada | - | 13 | 526 |  |
| R33 | 1 | rechazada | programacion | - | 77 |  |
| R34 | 1 | rechazada | programacion | 6 | 66 |  |
| R35 | 1 | rechazada | programacion | 21 | 1001 |  |
| R36 | 1 | aceptada | - | 17 | 108 |  |
| R37 | 1 | rechazada | programacion | 8 | 251 |  |
| R38 | 1 | rechazada | programacion | - | 78 |  |
| R39 | 1 | rechazada | programacion | 12 | 66 |  |
| R40 | 1 | aceptada | - | 13 | 89 |  |
