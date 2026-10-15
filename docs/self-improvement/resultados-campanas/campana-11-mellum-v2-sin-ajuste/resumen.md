**Resultado:** 1 aceptadas de 10 (mínimo SI-4: 5) → **NO-GO**

Mellum 2.1 12B (Q4_K_M) servido por soso en la GPU del host, T=0, agente v2 (`mod.rs` 25484f48…, `herramientas.rs` 9e7f908d…), estimador de contexto a 3 caracteres/token (el recorte nunca se activó). Banco R31–R40 (`campaign-11.json`).

| tarea | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|
| R31 | error | protocolo | 30 | 855 | error: entorno: el intento no terminó: LimiteHerramientas |
| R32 | rechazada | programacion | 9 | 592 |  |
| R33 | bloqueada | protocolo | 5 | 417 | el candidato no cambió nada |
| R34 | rechazada | programacion | 15 | 577 |  |
| R35 | error | infraestructura | 19 | 780 | error: entorno: el intento no terminó: ErrorModelo entorno: el servidor contestó Some(422): {"error":{"message |
| R36 | error | infraestructura | 20 | 748 | error: entorno: el intento no terminó: ErrorModelo entorno: el servidor contestó Some(422): {"error":{"message |
| R37 | rechazada | programacion | 25 | 1130 |  |
| R38 | bloqueada | protocolo | 10 | 381 | el candidato no cambió nada |
| R39 | bloqueada | protocolo | 6 | 452 | el candidato no cambió nada |
| R40 | aceptada | - | 14 | 336 |  |
