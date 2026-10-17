**Resultado:** 2 aceptadas de 10 a la primera (ronda 1) y **4 de 10 con hasta 3 rondas**.

Qwen3-Coder-30B-A3B (Q4_K/Q8_0, 128 expertos) servido por soso con **flujo de expertos** en la GPU del host (12 de 48 capas con expertos residentes, el resto por un pool con LRU), agente v3, banco R31–R40, T=0 en la ronda 1 y T=0,7 en las siguientes.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 2 | rechazada | programacion | 24 | 647 |  |
| R31 | 3 | rechazada | programacion | 24 | 396 |  |
| R31 | 1 | rechazada | programacion | 16 | 468 |  |
| R32 | 2 | rechazada | programacion | 10 | 536 |  |
| R32 | 3 | aceptada | - | 9 | 617 |  |
| R32 | 1 | rechazada | programacion | 14 | 645 |  |
| R33 | 2 | rechazada | programacion | 17 | 417 |  |
| R33 | 3 | rechazada | programacion | 17 | 415 |  |
| R33 | 1 | rechazada | programacion | 11 | 440 |  |
| R34 | 2 | rechazada | programacion | 11 | 182 |  |
| R34 | 3 | rechazada | programacion | 11 | 182 |  |
| R34 | 1 | rechazada | programacion | 11 | 180 |  |
| R35 | 2 | rechazada | programacion | 12 | 259 |  |
| R35 | 3 | aceptada | - | 12 | 259 |  |
| R35 | 1 | rechazada | programacion | 18 | 582 |  |
| R36 | 1 | aceptada | - | 8 | 252 |  |
| R37 | 2 | rechazada | programacion | 19 | 301 |  |
| R37 | 3 | rechazada | programacion | 10 | 229 |  |
| R37 | 1 | rechazada | programacion | 8 | 199 |  |
| R38 | 2 | rechazada | programacion | ? | 77 |  |
| R38 | 3 | rechazada | programacion | ? | 77 |  |
| R38 | 1 | error | protocolo | 30 | 581 | error: entorno: el intento no terminó: LimiteHerramientas |
| R39 | 2 | error | protocolo | 30 | 419 | error: entorno: el intento no terminó: LimiteHerramientas |
| R39 | 3 | error | protocolo | 30 | 371 | error: entorno: el intento no terminó: LimiteHerramientas |
| R39 | 1 | error | protocolo | 30 | 376 | error: entorno: el intento no terminó: LimiteHerramientas |
| R40 | 1 | aceptada | - | 23 | 524 |  |
