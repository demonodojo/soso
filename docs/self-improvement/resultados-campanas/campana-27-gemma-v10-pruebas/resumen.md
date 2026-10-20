**Resultado:** 1 aceptada de 10 (R40), una ronda.

Gemma 4 12B por soso (GPU del host), agente v10, **prompt de «pruebas propias»** (`SISTEMA_AGENTE=pruebas`), banco R31–R40 con `campaign-12.json` (60 herramientas y **40 turnos efectivos** (el arnés recortaba `--max-turnos` a 40 hasta el binario v12)). Tres intentos acaban en `LimitePasos` (R31, R32, R38: bucles del modelo con las guardas ya activas), dos cierran sin editar nada (R35, R36). Las tandas anteriores de esta campaña se descartaron por defectos del agente (ver `campana.md`, «Campañas 22–29»).

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 1 | error | protocolo | 29 | 848 | error: entorno: el intento no terminó: LimitePasos |
| R32 | 1 | error | protocolo | 40 | 1976 | error: entorno: el intento no terminó: LimitePasos |
| R33 | 1 | rechazada | programacion | - | 78 |  |
| R34 | 1 | rechazada | programacion | 22 | 1159 |  |
| R35 | 1 | bloqueada | protocolo | 11 | 110 | el candidato no cambió nada |
| R36 | 1 | bloqueada | protocolo | 10 | 195 | el candidato no cambió nada |
| R37 | 1 | rechazada | programacion | 15 | 189 |  |
| R38 | 1 | error | protocolo | 36 | 1240 | error: entorno: el intento no terminó: LimitePasos |
| R39 | 1 | rechazada | programacion | 9 | 83 |  |
| R40 | 1 | aceptada | - | 10 | 134 |  |
