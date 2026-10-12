**Resultado:** 2 aceptadas de 10 (mínimo SI-4: 5) → **NO-GO**

- no cumple: 2 aceptadas de 10: se exigen al menos 5

| tarea | resultado | fallo de | intentos | herramientas | tokens | segundos | inyección |
|---|---|---|---|---|---|---|---|
| R01 | Error | Protocolo | 1 | 3 | 5797 | 519 | - |
| R02 | Error | Protocolo | 2 | ? | ? | 1051 | agente matado (kill -9) y coordinador reiniciado (recuperada) |
| R03 | Error | Protocolo | 1 | 5 | 20283 | 644 | - |
| R04 | Bloqueada | Protocolo | 1 | 4 | 33483 | 814 | - |
| R05 | Error | Protocolo | 1 | 4 | 12866 | 868 | - |
| R06 | Aceptada | - | 1 | 5 | 17043 | 475 | - |
| R07 | Error | Protocolo | 2 | ? | ? | 1259 | agente matado (kill -9) y coordinador reiniciado (recuperada) |
| R08 | Aceptada | - | 1 | 4 | 17685 | 328 | - |
| R09 | Rechazada | Programacion | 1 | 6 | 42442 | 1041 | - |
| R10 | Rechazada | Programacion | 1 | 4 | 17873 | 626 | - |

**Por mejora aceptada:** 3812 s, ? tokens, ? herramientas, 6.00 intentos (sobre todo lo gastado).

**Fallos por responsable:** {Protocolo: 6, Programacion: 2}
- sin medir: tokens: no se midió en R02
- sin medir: herramientas: no se midió en R02
