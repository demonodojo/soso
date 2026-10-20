**Resultado:** 3 aceptadas de 10 a la primera (R32, R36, R40) y **3 de 10 con hasta 3 rondas**: las rondas 2 y 3 (T=0,7) no añaden ninguna.

Gemma 4 12B por soso (GPU del host), agente v11 (umbral de 8 llamadas bloqueadas), prompt estándar, banco R31–R40 con `campaign-12.json` (60 herramientas y **40 turnos efectivos** (el arnés recortaba `--max-turnos` a 40 hasta el binario v12)), T=0 en la ronda 1 y T=0,7 en las siguientes. R35 agota los turnos en las rondas 2 y 3 (`LimitePasos`, 53 y 38 min: lecturas grandes y lentas, no un bucle de las guardas).

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 2 | bloqueada | protocolo | 11 | 97 | el candidato no cambió nada |
| R31 | 3 | rechazada | programacion | 11 | 76 |  |
| R31 | 1 | rechazada | programacion | 17 | 101 |  |
| R32 | 1 | aceptada | - | 13 | 503 |  |
| R33 | 2 | rechazada | programacion | - | 77 |  |
| R33 | 3 | rechazada | programacion | - | 78 |  |
| R33 | 1 | rechazada | programacion | 15 | 76 |  |
| R34 | 2 | rechazada | programacion | 13 | 123 |  |
| R34 | 3 | rechazada | programacion | 13 | 123 |  |
| R34 | 1 | rechazada | programacion | 6 | 69 |  |
| R35 | 2 | error | protocolo | 39 | 3195 | error: entorno: el intento no terminó: LimitePasos |
| R35 | 3 | error | protocolo | 40 | 2253 | error: entorno: el intento no terminó: LimitePasos |
| R35 | 1 | rechazada | programacion | 24 | 632 |  |
| R36 | 1 | aceptada | - | 17 | 108 |  |
| R37 | 2 | rechazada | programacion | 16 | 210 |  |
| R37 | 3 | rechazada | programacion | 16 | 212 |  |
| R37 | 1 | rechazada | programacion | 8 | 168 |  |
| R38 | 2 | rechazada | programacion | - | 77 |  |
| R38 | 3 | rechazada | programacion | - | 77 |  |
| R38 | 1 | rechazada | programacion | - | 78 |  |
| R39 | 2 | rechazada | programacion | 22 | 108 |  |
| R39 | 3 | rechazada | programacion | 22 | 106 |  |
| R39 | 1 | rechazada | programacion | 12 | 67 |  |
| R40 | 1 | aceptada | - | 16 | 94 |  |
