# T30 — Medir servicio e inferencia en una máquina física identificada

**Hito:** SI-5 · **Tipo:** Integración de hardware · **Estado:** pendiente.  
**Dependencias:** [T19](T19-qemu-e2e.md), [T29](T29-campana.md)

## Objetivo y entrega

Diez tareas atendidas sin caída del servicio ni agotamiento progresivo; fallos del modelo y del sistema separados.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [docs/HW-MATRIX.md](../../docs/HW-MATRIX.md)
- [docs/hw-matrix.json](../../docs/hw-matrix.json)
- [docs/DIAGNOSTICO-GB205-2026-09-15.md](../../docs/DIAGNOSTICO-GB205-2026-09-15.md)
- [docs/DIAGNOSTICO-ROG-2026-09-15.md](../../docs/DIAGNOSTICO-ROG-2026-09-15.md)
- [docs/WIFI-OPERATIVA.md](../../docs/WIFI-OPERATIVA.md)

## Archivos que se pueden cambiar

Añadir el informe de hardware a `crates/soso-improve-core` con su orden en `tools/soso-improve`, pruebas en el mismo crate e informe por equipo en `docs/self-improvement/hardware/`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** esta ficha pedía Python; se reescribió el 16 de septiembre de 2026
> a Rust, porque el objetivo del plan es que todo pueda correr **dentro de soso**
> y soso no tiene intérprete de Python. Ver
> [seguimiento/T01.md](seguimiento/T01.md).

## Pasos

1. Leer skills soso-gpu/soso-wifi/soso-live solo para los subsistemas que se prueben. Identificar equipo, NIC, IP, kernel, modelo, backend efectivo y medio de arranque.
2. Comprobar autenticación y acceso de red configurado. Repetir T14 y primera mejora contra el endpoint físico, con VM candidata independiente.
3. Medir diez tareas seguidas, cold/warm, RAM/VRAM, salud después de cada una, pérdida/reconexión de red y crecimiento de memoria respecto a límites fijados antes de la campaña.
4. Comparar CPU y GPU si ambas están operativas con los mismos casos de corrección. Un GSP arrancado o pool disponible no basta para marcar inferencia GPU.
5. Generar informe y actualizar matriz solo a partir de logs de ese equipo; conservar casos sin hardware como no evaluados.

## Comprobación

`cargo test -p soso-improve-core -p soso-improve` con logs guardados; campaña real aparte. El parser no debe convertir ausencia de datos en cero fallos.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si red/GPU están bloqueadas, entregar diagnóstico específico y seguir por un backend ya probado; no certificar silicio con QEMU.

Entregar `target/self-improvement/tasks/T30/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

