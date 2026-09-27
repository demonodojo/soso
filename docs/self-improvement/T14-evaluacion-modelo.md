# T14 — Medir calidad y fijar presupuestos antes de usar el agente

**Hito:** SI-0 / SI-1 · **Tipo:** Implementación de evaluación e integración · **Estado:** hecha (2026-09-23) · **Resultado del perfil elegido (7B): GO**.

El 27-sep-2026 el perfil pasó a ser `qwen2.5-coder-7b`. Su campaña es GO 10/10, cobertura `completa` (30/30, exit 0). El 3B, candidato anterior, sigue en NO-GO 8/10. Detalle en [seguimiento/T14.md](seguimiento/T14.md).

**Dependencias:** [T02](T02-banco.md), [T03](T03-perfil-modelo.md), [T13](T13-servidor-host.md), [T48](T48-reloj-red.md)

## Objetivo y entrega

Modelo apto demostrado y presupuestos medidos, o no-go justificado con la siguiente corrección concreta. Un no-go no satisface la dependencia funcional de T16/T22.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2–C3, C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [crates/soso-llm-core/examples/hostrun.rs](../../crates/soso-llm-core/examples/hostrun.rs)
- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Añadir la evaluación a `crates/soso-improve-core` (lógica portable) con su orden en `tools/soso-improve` y `user/soso-improve` y pruebas en el mismo crate, más `docs/self-improvement/evaluacion.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** esta ficha pedía Python; se reescribió el 16 de septiembre de 2026
> a Rust, porque el objetivo del plan es que todo pueda correr **dentro de soso**
> y soso no tiene intérprete de Python. Ver
> [seguimiento/T01.md](seguimiento/T01.md).

## Pasos

1. Consumir el banco T02 y un endpoint explícito. Medir con el reloj monotónico inyectado de T48 (adaptadores host y soso): primer byte útil, primer token, fin, tokens reales, memoria reportada y fallos.
2. Ejecutar cada caso tres veces con semillas registradas. Separar frío/caliente y corrección/velocidad; incluir todos los intentos en el denominador.
3. Usar verificadores reservados fuera del contexto del modelo. Ejercitar tool request → resultado de herramienta → siguiente petición.
4. Probar contexto representativo de OpenCode y el caso de 8 Ki tokens si cabe. Fijar límite de salida, timeouts finitos y presupuesto por tarea en model-lock.
5. Generar informe go/no-go con 10/10 casos de protocolo y ≥8/10 microtareas por ejecución según el plan, además de variabilidad. Repetir con otro perfil si falla, sin cambiar el banco durante la comparación.

## Comprobación

`cargo test -p soso-improve-core -p soso-improve`. Backend simulado con éxito, timeout, usage ausente y salida errónea; comprobar que no se cuentan como éxito. Adjuntar campaña real por separado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados (lógica portable y pruebas).
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado: **NO-GO justificado**, con la corrección concreta que sigue.

Entregado el 23-sep-2026: `crates/soso-improve-core/src/evaluacion.rs` con el
criterio del plan convertido en código —todos los intentos en el denominador,
cinco desenlaces distintos, `sin-uso` que no cuenta, umbrales explícitos y
presupuestos derivados de lo medido— y [evaluacion.md](evaluacion.md) como
criterio citable. `cargo test -p soso-improve-core -p soso-improve` verde con
backend simulado. Resumen en [seguimiento/T14.md](seguimiento/T14.md).

**Campañas y atribución actualizadas el 26-sep-2026.** El primer intento
(23-sep) dio 6/10. Tras T59/T60, la campaña completa del 25-sep dio **8/10**,
0 inestables: Q04/Q07 fallan con el 3B; mediana 50 058 ms, máximo 287 100 ms.
La atribución inicial de los cuatro fallos al servicio quedó superada por T59.

La comparación 7B ya fue elegida y ejecutada el 26-sep, pero perdió conexión
tras Q05. Su 4/10 agregado no acredita calidad completa: Q04 sí pasa y Q07
no se midió. **[T74](T74-campana-7b-interrumpida.md) es el siguiente trabajo**.
Informes separados en `target/self-improvement/tasks/T14/campana-t14-3b.json`
y `campana-t14-7b.json`; no usar el nombre canónico T19 como prueba del 3B.
El perfil C1 sigue siendo el 3B hasta comparación válida.

Como la ficha exige, **un no-go no habilita a [T16](T16-servicio-guest.md) ni a
[T22](T22-primera-mejora.md)**, y SI-2 no cierra con esto.

**Límites.** Sólo se evaluó la clase protocolo: las microtareas necesitan
compilador en el guest (T40/T41), y el umbral se puso a 0 para no fabricar un
no-go por una clase que ni se intentó. `native_validation` queda en partial: la
campaña la lanza el host contra el guest, y ejecutarla desde soso es
[T49](T49-pruebas-guest.md).

Si el modelo no usa herramientas, no pasar a una demo que ejecute comandos inventados por el adaptador.

Entregar `target/self-improvement/tasks/T14/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **parcial** (campaña host contra inferencia guest). Condiciones adicionales: [T16](T16-servicio-guest.md), [T41](T41-cargo-offline.md), [T48](T48-reloj-red.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).

## Revisión del 26-sep-2026

Estado y siguiente paso sincronizados con el catálogo; ver
[seguimiento/T14.md](seguimiento/T14.md) y
[revisión del plan](seguimiento/REVISION-2026-09-26.md).
