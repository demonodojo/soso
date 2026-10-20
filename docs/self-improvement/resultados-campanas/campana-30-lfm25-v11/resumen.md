**Resultado:** 2 aceptadas de 10 (R36, R40), una ronda.

LFM2.5-24B-A2B (Liquid AI, MoE híbrido de convolución corta y atención, Q4_0 QAD de 13,6 GB) servido por **llama.cpp** (commit `5aeac37`, CUDA sm_120, 16 hilos, contexto 32 768) en la GPU del host; agente v11, prompt estándar, banco R31–R40 con `campaign-12.json`, `MAX_TOKENS=8192` (el modelo siempre piensa) y 40 turnos efectivos (ver el defecto de `--max-turnos` en `campana.md`). Prefill ≈1500 tok/s y decodificación ≈65–80 tok/s. Tres intentos acaban en `LimitePasos` (R35, R37, R39).

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R31 | 1 | bloqueada | protocolo | 18 | 730 | el candidato no cambió nada |
| R32 | 1 | rechazada | programacion | 21 | 579 |  |
| R33 | 1 | rechazada | programacion | 27 | 496 |  |
| R34 | 1 | rechazada | programacion | 16 | 578 |  |
| R35 | 1 | error | protocolo | 39 | 485 | error: entorno: el intento no terminó: LimitePasos |
| R36 | 1 | aceptada | - | 22 | 701 |  |
| R37 | 1 | error | protocolo | 38 | 593 | error: entorno: el intento no terminó: LimitePasos |
| R38 | 1 | rechazada | programacion | 21 | 368 |  |
| R39 | 1 | error | protocolo | 39 | 570 | error: entorno: el intento no terminó: LimitePasos |
| R40 | 1 | aceptada | - | 22 | 279 |  |
