**Resultado:** 2 aceptadas de 10 a la primera (ronda 1) y **3 de 10 con hasta 3 rondas** (mínimo SI-4: 5 a un intento; esta cifra con reintentos no es comparable con las anteriores campañas) .

Gemma 4 12B servido por soso en la GPU del host, banco R31–R40 (`campaign-11.json`), T=0 en la ronda 1; agente v3, 3 rondas, reintentos a T=0; R39/R40 caen en un bucle de `cargo test` en verde.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 2 | rechazada | programacion | 9 | 61 |  |
| R31 | 3 | rechazada | programacion | 9 | 61 |  |
| R31 | 1 | rechazada | programacion | 7 | 41 |  |
| R32 | 2 | error | protocolo | 30 | 455 | error: entorno: el intento no terminó: LimiteHerramientas |
| R32 | 3 | error | protocolo | 30 | 457 | error: entorno: el intento no terminó: LimiteHerramientas |
| R32 | 1 | error | protocolo | 30 | 1278 | error: entorno: el intento no terminó: LimiteHerramientas |
| R33 | 2 | rechazada | programacion | ? | 78 |  |
| R33 | 3 | rechazada | programacion | 7 | 76 |  |
| R33 | 1 | rechazada | programacion | ? | 77 |  |
| R34 | 1 | aceptada | - | 16 | 187 |  |
| R35 | 2 | aceptada | - | 15 | 311 |  |
| R35 | 1 | rechazada | programacion | 18 | 475 |  |
| R36 | 1 | aceptada | - | 7 | 51 |  |
| R37 | 2 | rechazada | programacion | 18 | 88 |  |
| R37 | 3 | rechazada | programacion | 13 | 74 |  |
| R37 | 1 | rechazada | programacion | 6 | 54 |  |
| R38 | 2 | rechazada | programacion | ? | 77 |  |
| R38 | 3 | rechazada | programacion | ? | 77 |  |
| R38 | 1 | rechazada | programacion | ? | 77 |  |
| R39 | 2 | error | protocolo | 30 | 69 | error: entorno: el intento no terminó: LimiteHerramientas |
| R39 | 3 | error | protocolo | 30 | 69 | error: entorno: el intento no terminó: LimiteHerramientas |
| R39 | 1 | rechazada | programacion | 8 | 65 |  |
| R40 | 2 | error | protocolo | 30 | 101 | error: entorno: el intento no terminó: LimiteHerramientas |
| R40 | 3 | error | protocolo | 30 | 101 | error: entorno: el intento no terminó: LimiteHerramientas |
| R40 | 1 | error | protocolo | 30 | 105 | error: entorno: el intento no terminó: LimiteHerramientas |
