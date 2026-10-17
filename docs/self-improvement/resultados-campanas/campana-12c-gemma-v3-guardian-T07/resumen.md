**Resultado:** 3 aceptadas de 10 a la primera (ronda 1) y **4 de 10 con hasta 3 rondas** (mínimo SI-4: 5 a un intento; esta cifra con reintentos no es comparable con las anteriores campañas) .

Gemma 4 12B servido por soso en la GPU del host, banco R31–R40 (`campaign-11.json`), T=0 en la ronda 1; agente v3 + guardián «ya salió bien desde la última edición», reintentos a T=0,7.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 2 | rechazada | programacion | 17 | 97 |  |
| R31 | 3 | rechazada | programacion | 12 | 110 |  |
| R31 | 1 | rechazada | programacion | 7 | 40 |  |
| R32 | 2 | aceptada | - | 9 | 338 |  |
| R32 | 1 | rechazada | programacion | 26 | 833 |  |
| R33 | 2 | rechazada | programacion | ? | 77 |  |
| R33 | 3 | rechazada | programacion | 7 | 76 |  |
| R33 | 1 | rechazada | programacion | ? | 77 |  |
| R34 | 1 | aceptada | - | 16 | 173 |  |
| R35 | 2 | bloqueada | protocolo | 26 | 377 | el candidato no cambió nada |
| R35 | 3 | bloqueada | protocolo | 26 | 372 | el candidato no cambió nada |
| R35 | 1 | bloqueada | protocolo | 14 | 301 | el candidato no cambió nada |
| R36 | 1 | aceptada | - | 7 | 54 |  |
| R37 | 2 | rechazada | programacion | 16 | 187 |  |
| R37 | 3 | rechazada | programacion | 14 | 123 |  |
| R37 | 1 | rechazada | programacion | 10 | 60 |  |
| R38 | 2 | rechazada | programacion | ? | 77 |  |
| R38 | 3 | rechazada | programacion | ? | 77 |  |
| R38 | 1 | rechazada | programacion | ? | 77 |  |
| R39 | 2 | rechazada | programacion | 8 | 66 |  |
| R39 | 3 | rechazada | programacion | 8 | 66 |  |
| R39 | 1 | rechazada | programacion | 8 | 55 |  |
| R40 | 1 | aceptada | - | 12 | 84 |  |
