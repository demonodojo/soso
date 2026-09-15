# T02 — Definir los casos y sus verificadores

**Hito:** SI-0 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

25 entradas con aceptación automatizable y partición reproducible; las cinco tareas reales tienen evidencia del estado previo.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [crates/soso-llm-core/src/chat.rs](../../crates/soso-llm-core/src/chat.rs)
- [crates/soso-llm-core/tests/arch_ext.rs](../../crates/soso-llm-core/tests/arch_ext.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)

## Archivos que se pueden cambiar

Crear `tests/self-improvement/cases/`, `case.schema.json` y `test_cases.py` en ese directorio padre.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Definir JSON por caso: id, clase, partición desarrollo/reservado, entrada, resultado observable, comprobador argv, timeout y recursos. Validar IDs únicos y rutas relativas.
2. Crear 10 casos de programación con expectativas explícitas: UTF-8, escape JSON, longitud HTTP, saturación de contador, ruta relativa, error de E/S, fin de iterador, orden estable, valor límite y liberación de recurso. Usar programas mínimos aislados y entradas/salidas esperadas, sin afirmar bugs del proyecto.
3. Crear 10 casos de protocolo: texto simple, system+user, dos turnos, llamada+resultado, contenido null, Unicode fragmentado, argumentos fragmentados, nombre desconocido, JSON incompleto y contexto excesivo.
4. Seleccionar 5 tareas del repo a partir de reproducción o una mejora especificada. Cada ficha incluye base, rutas y aserción de aceptación. No inventar defectos presentes ni entregar como solución un parche preescrito.
5. Separar solucionarios/verificadores reservados del paquete visible al agente: el lanzador copiará solo entradas y fuentes permitidas. Mantener la distribución y los umbrales inmutables durante cada campaña.

## Comprobación

`python3 -m unittest discover -s tests/self-improvement -p test_cases.py`. Afirmar 10/10/5 casos, IDs únicos, verificadores ejecutables y detección tanto de una solución correcta como de una incorrecta.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Una tarea real sin reproducción o comportamiento especificado queda excluida y debe sustituirse antes de cerrar el banco.

Entregar `target/self-improvement/tasks/T02/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

