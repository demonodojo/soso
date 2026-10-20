**Resultado:** repetición de R35, R37 y R39 con el agente v12 y `--max-turnos 120` (**que en realidad eran 40**: el modo `tarea` no leía el flag hasta el binario v18): R37 y R39 acaban **rechazadas** y R35 vuelve a agotar los turnos; la diferencia con la campaña 30 es variación entre ejecuciones, no más turnos. Ninguna aceptada: el LFM sigue en 2/10 en el banco.

| tarea | ronda | resultado | fallo de | herramientas | segundos | nota |
|---|---|---|---|---|---|---|
| R35 | 1 | error | protocolo | 40 | 686 | error: entorno: el intento no terminó: LimitePasos |
| R37 | 1 | rechazada | programacion | 30 | 296 |  |
| R39 | 1 | rechazada | programacion | 37 | 1470 |  |
