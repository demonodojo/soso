# T25 — Ejecutar OpenCode con límites y logs

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T21](T21-opencode-contrato.md), [T24](T24-checkout.md), [T47](T47-procesos-nativos.md), [T48](T48-reloj-red.md)

## Objetivo y entrega

Run deja estado y logs aun si OpenCode muere; ninguna herramienta fuera de presupuesto inicia otra iteración.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [crates/soso-improve-core/src/lib.rs](../../crates/soso-improve-core/src/lib.rs)
- [tools/soso-improve/src/main.rs](../../tools/soso-improve/src/main.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear runner/events en `crates/soso-improve-core` y pruebas compartidas; conectar `run` en los adaptadores host y guest mediante T47/T48.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Ejecutar OpenCode como proceso con argv/cwd/env mínimos explícitos. El runner recibe un backend de aislamiento configurado; sin él no habilitar tareas que ejecutan código arbitrario del candidato.
2. Consumir stdout JSON incremental y stderr por separado sin deadlocks; conservar bytes originales y producir eventos normalizados usando fixtures T21.
3. Contar herramientas, intentos y tokens reportados, evitando duplicar totales acumulados. Cancelar por límite; medir timeout con reloj monotónico.
4. Tras timeout limpiar procesos y descendientes mediante el mecanismo acreditado de T47 y esperar su terminación. No asumir grupos POSIX; si falta contención usar instancia candidata dedicada. Identificar ejecución con más que PID.
5. Limitar tamaño de logs y salida enviada al modelo; al exceder límite guardar evento y terminar de manera explícita. No bloquear esperando una aprobación interactiva: registrar acción fuera de política.

## Comprobación

`cargo test -p soso-improve-core --test runner`. Ejecutable falso controlado: JSON dividido, stderr abundante, proceso hijo, bloqueo, exceso de llamadas, uso acumulado y salida incompleta. Después humo con OpenCode real.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

La falta de soporte fiable para un límite se declara y se suple con un límite verificable de tiempo/herramientas; no inventar consumo.

Entregar `target/self-improvement/tasks/T25/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). T47 aporta semántica real de procesos y T48 reloj. La ejecución de código candidato requiere instancia separada o contención acreditada; no asumir sandbox, grupos de procesos POSIX ni aislamiento por directorio.

Validación nativa: **pendiente**. Condiciones adicionales: [T35](T35-opencode-nativo.md), [T47](T47-procesos-nativos.md), [T48](T48-reloj-red.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
