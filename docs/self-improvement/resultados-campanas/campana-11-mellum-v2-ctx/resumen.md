**Resultado:** 0 aceptadas de 10 (mínimo SI-4: 5) → **NO-GO**

Mellum 2.1 12B (Q4_K_M) servido por soso en la GPU del host, T=0, agente v2 con `chars_por_token = 1.5` (`SOSO_AGENTE_CHARS_POR_TOKEN=1.5`). Banco R31–R40 (`campaign-11.json`).

| tarea | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|
| R31 | error | protocolo | 30 | 627 | error: entorno: el intento no terminó: LimiteHerramientas |
| R32 | rechazada | programacion | 11 | 642 |  |
| R33 | bloqueada | protocolo | 5 | 371 | el candidato no cambió nada |
| R34 | rechazada | programacion | 24 | 929 |  |
| R35 | rechazada | programacion | 15 | 357 |  |
| R36 | rechazada | programacion | 23 | 1052 |  |
| R37 | rechazada | programacion | 9 | 534 |  |
| R38 | bloqueada | protocolo | 10 | 398 | el candidato no cambió nada |
| R39 | bloqueada | protocolo | 6 | 508 | el candidato no cambió nada |
| R40 | error | protocolo | 30 | 538 | error: entorno: el intento no terminó: LimiteHerramientas |
