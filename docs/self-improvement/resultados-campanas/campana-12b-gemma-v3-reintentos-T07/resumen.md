**Resultado:** 1 aceptadas de 10 a la primera (ronda 1) y **5 de 10 con hasta 3 rondas** (mínimo SI-4: 5 a un intento; esta cifra con reintentos no es comparable con las anteriores campañas) .

Gemma 4 12B servido por soso en la GPU del host, banco R31–R40 (`campaign-11.json`), T=0 en la ronda 1; agente v3 + guardián de repetición consecutiva, reintentos a T=0,7; R40 sigue en bucle (alternaba dos comandos).

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 2 | rechazada | programacion | 14 | 101 |  |
| R31 | 3 | rechazada | programacion | 12 | 139 |  |
| R31 | 1 | rechazada | programacion | 7 | 40 |  |
| R32 | 2 | rechazada | programacion | 18 | 341 |  |
| R32 | 3 | aceptada | - | 9 | 330 |  |
| R32 | 1 | error | protocolo | 30 | 1268 | error: entorno: el intento no terminó: LimiteHerramientas |
| R33 | 2 | rechazada | programacion | ? | 77 |  |
| R33 | 3 | rechazada | programacion | ? | 77 |  |
| R33 | 1 | rechazada | programacion | 8 | 76 |  |
| R34 | 2 | aceptada | - | 7 | 68 |  |
| R34 | 1 | rechazada | programacion | 9 | 81 |  |
| R35 | 2 | aceptada | - | 15 | 93 |  |
| R35 | 1 | rechazada | programacion | 18 | 461 |  |
| R36 | 1 | aceptada | - | 7 | 51 |  |
| R37 | 2 | rechazada | programacion | 15 | 171 |  |
| R37 | 3 | aceptada | - | 22 | 312 |  |
| R37 | 1 | rechazada | programacion | 7 | 53 |  |
| R38 | 2 | rechazada | programacion | ? | 77 |  |
| R38 | 3 | rechazada | programacion | ? | 77 |  |
| R38 | 1 | rechazada | programacion | ? | 77 |  |
| R39 | 2 | rechazada | programacion | 8 | 68 |  |
| R39 | 3 | rechazada | programacion | 8 | 66 |  |
| R39 | 1 | rechazada | programacion | 8 | 76 |  |
| R40 | 2 | error | protocolo | 30 | 127 | error: entorno: el intento no terminó: LimiteHerramientas |
| R40 | 3 | error | protocolo | 30 | 126 | error: entorno: el intento no terminó: LimiteHerramientas |
| R40 | 1 | error | protocolo | 30 | 107 | error: entorno: el intento no terminó: LimiteHerramientas |
