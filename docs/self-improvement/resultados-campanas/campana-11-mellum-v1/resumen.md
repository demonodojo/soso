**Resultado:** 1 aceptadas de 10 (mínimo SI-4: 5) → **NO-GO**

Mellum 2.1 12B (Q4_K_M) servido por soso en la GPU del host, T=0, agente v1 (`mod.rs` 986a8692…, `herramientas.rs` b4ea58ae…), contexto estimado a 3 caracteres/token. Banco R31–R40 (`campaign-11.json`).

| tarea | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|
| R31 | error | protocolo | 30 | 750 | error: entorno: el intento no terminó: LimiteHerramientas |
| R32 | rechazada | programacion | 9 | 589 |  |
| R33 | bloqueada | protocolo | 5 | 415 | el candidato no cambió nada |
| R34 | rechazada | programacion | 14 | 468 |  |
| R35 | error | infraestructura | 19 | 779 | error: entorno: el intento no terminó: ErrorModelo entorno: el servidor contestó Some(422): {"error":{"message |
| R36 | error | infraestructura | 20 | 746 | error: entorno: el intento no terminó: ErrorModelo entorno: el servidor contestó Some(422): {"error":{"message |
| R37 | rechazada | programacion | 9 | 521 |  |
| R38 | bloqueada | protocolo | 17 | 672 | el candidato no cambió nada |
| R39 | bloqueada | protocolo | 6 | 452 | el candidato no cambió nada |
| R40 | aceptada | - | 13 | 332 |  |
