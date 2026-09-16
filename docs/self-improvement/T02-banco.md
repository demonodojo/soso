# T02 — Definir los casos y sus verificadores

**Hito:** SI-0 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** completada (15 de septiembre de 2026; resumen en [seguimiento/T02.md](seguimiento/T02.md), evidencia en `target/self-improvement/tasks/T02/resultado.md`).

**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

25 entradas con aceptación automatizable y partición reproducible; las cinco tareas reales tienen evidencia del estado previo.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [crates/soso-llm-core/src/chat.rs](../../crates/soso-llm-core/src/chat.rs)
- [crates/soso-llm-core/tests/arch_ext.rs](../../crates/soso-llm-core/tests/arch_ext.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)

## Archivos que se pueden cambiar

Crear `tests/self-improvement/cases/` y `case.schema.json`; la lógica del banco y sus verificadores van en `crates/soso-improve-core`, con las pruebas en `tools/soso-improve/tests/banco.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

> **Lenguaje:** igual que T01, el banco se implementó en Python y se portó a Rust
> el 16 de septiembre de 2026. Los 25 casos JSON no cambiaron: eran datos neutros.
> Ver [seguimiento/T02.md](seguimiento/T02.md).

## Pasos

1. Definir JSON por caso: id, clase, partición desarrollo/reservado, entrada, resultado observable, comprobador argv, timeout y recursos. Validar IDs únicos y rutas relativas.
2. Crear 10 casos de programación con expectativas explícitas: UTF-8, escape JSON, longitud HTTP, saturación de contador, ruta relativa, error de E/S, fin de iterador, orden estable, valor límite y liberación de recurso. Usar programas mínimos aislados y entradas/salidas esperadas, sin afirmar bugs del proyecto.
3. Crear 10 casos de protocolo: texto simple, system+user, dos turnos, llamada+resultado, contenido null, Unicode fragmentado, argumentos fragmentados, nombre desconocido, JSON incompleto y contexto excesivo.
4. Seleccionar 5 tareas del repo a partir de reproducción o una mejora especificada. Cada ficha incluye base, rutas y aserción de aceptación. No inventar defectos presentes ni entregar como solución un parche preescrito.
5. Separar solucionarios/verificadores reservados del paquete visible al agente: el lanzador copiará solo entradas y fuentes permitidas. Mantener la distribución y los umbrales inmutables durante cada campaña.

## Comprobación

`cargo test -p soso-improve-core -p soso-improve`. Afirmar 10/10/5 casos, IDs únicos, verificadores ejecutables y detección tanto de una solución correcta como de una incorrecta.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Una tarea real sin reproducción o comportamiento especificado queda excluida y debe sustituirse antes de cerrar el banco.

Entregar `target/self-improvement/tasks/T02/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Cierre histórico conservado. Validar banco/protocolo con T49; programa y repo requieren T40/T41 y referencias portables T50. Mantener huella, partición reservada y denominador; completar evidencia en T51.

Validación nativa: **pendiente**. Condiciones adicionales: [T45](T45-cli-capacidades.md), [T49](T49-pruebas-guest.md), [T40](T40-compilador-nativo.md), [T41](T41-cargo-offline.md), [T50](T50-cambios-contenido.md), [T51](T51-aceptacion-circuito-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
