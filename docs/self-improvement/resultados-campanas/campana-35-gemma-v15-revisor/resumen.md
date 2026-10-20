**Resultado:** 2 aceptadas de 10 (R36, R40), una ronda, 80 turnos pedidos (**40 efectivos**: el modo `tarea` no leía `--max-turnos` hasta el binario v18).

Gemma 4 12B por soso, agente v15 con `REVISOR=1`: antes de terminar, una petición aparte al mismo modelo (sin la conversación del agente ni herramientas; sólo `TASK.md` y el diff) lista hasta cinco fallos probables con una entrada concreta; esa lista vuelve al agente. Señala huecos plausibles, pero el agente los parchea mal y se enreda arreglando sus propios arreglos: R31 y R32 (rechazadas en ~100 s sin revisor) gastan los 80 turnos y acaban en `LimitePasos`.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 1 | error | protocolo | 34 | 999 | error: entorno: el intento no terminó: LimitePasos |
| R32 | 1 | error | protocolo | 35 | 1615 | error: entorno: el intento no terminó: LimitePasos |
| R33 | 1 | rechazada | programacion | - | 78 |  |
| R34 | 1 | rechazada | programacion | 23 | 399 |  |
| R35 | 1 | rechazada | programacion | 35 | 1385 |  |
| R36 | 1 | aceptada | - | 18 | 199 |  |
| R37 | 1 | rechazada | programacion | 13 | 169 |  |
| R38 | 1 | rechazada | programacion | - | 77 |  |
| R39 | 1 | rechazada | programacion | 22 | 293 |  |
| R40 | 1 | aceptada | - | 17 | 220 |  |
