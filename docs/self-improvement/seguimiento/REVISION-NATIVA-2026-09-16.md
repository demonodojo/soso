# Revisión de los planes para ejecución completa en soso

**Fecha:** 2026-09-16. **Alcance:** documentación; no implementación ni campaña.

## Evidencia inspeccionada

- Core no_std y adaptadores tools/user de soso-improve ya existen.
- Guest Procesos pierde límites de argv, mezcla canales y omite stdin/env/timeout; algunas órdenes imprimen fallo y devuelven éxito.
- Captura por contenido permite reconstrucción sin Git; referencias repo del host aún usan git apply.
- Las syscalls de procesos, reloj, rename y fsync existen; falta acreditar su semántica para el circuito.

## Cambios del plan

[NATIVO.md](../NATIVO.md) fija arquitectura, capacidades y cierre. Se añaden
T45–T51, se actualizan scopes/dependencias y se separa native_validation
del estado histórico. T01/T02 siguen done; su validación nativa está pending.
Los resúmenes previos y huellas del banco no se modifican.

## Próximo trabajo

T03 sigue habilitada para el perfil. T45 puede iniciarse para CLI/errores,
y T46 para persistencia; una ficha por sesión. T47–T50 completan mecanismos;
T51 exige campaña íntegra dentro de soso y habilita cierre T44.

No se ejecutaron builds, pruebas guest ni se certificó hardware en esta revisión.
La comprobación documental valida catálogo, dependencias, enlaces y sincronización.

## Verificación documental

Catálogo de 51 fichas validado: IDs únicos, estados/tipos/dependencias
sincronizados con fichas e índice, grafo de desarrollo sin ciclos y 609 enlaces
locales existentes. Comparación con HEAD confirma que estados y tracking de
T01/T02 se conservan. `git diff --check` pasó. No se ejecutaron pruebas de
runtime porque esta entrega solo cambia planes y guía de seguimiento.
