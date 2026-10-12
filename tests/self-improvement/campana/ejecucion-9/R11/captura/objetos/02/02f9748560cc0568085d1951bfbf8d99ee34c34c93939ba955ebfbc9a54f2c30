# T44 — Repetir tres mejoras con agente, modelo y build en soso

**Hito:** SI-7 · **Tipo:** Evaluación final · **Estado:** pendiente.  
**Dependencias:** [T29](T29-campana.md), [T43](T43-validacion-actualizacion-nativa.md)

## Objetivo y entrega

SI-7 se cierra solo con tres ciclos completos sin edición ni compilación manual en Linux. Actualizar estado del plan padre con enlaces a evidencia.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/campana-final.md` y manifiesto/artefactos de campaña; modificar solo rutas de las tres tareas aprobadas.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Fijar tres tareas con aceptación antes de iniciar; comprobar que modelo, OpenCode, fuentes, toolchain y empaquetadores están en soso.
2. Ejecutar ciclo completo por tarea: observar, editar, compilar, validar, arrancar candidato y establecer la siguiente base.
3. Comprobar que la compilación continúa con Forja inaccesible. La observación externa puede recoger logs, pero no editar ni compilar para rescatar una tarea.
4. Incluir una versión candidata fallida y verificar recuperación. Mantener intentos y resultados sin excluir fallos.
5. Publicar matriz final de ubicaciones y cobertura de pruebas; separar dependencias externas de observación de dependencias funcionales aún presentes.

## Comprobación

Tres mejoras de código aceptadas consecutivamente con hash de cada base y build, boot de cada versión y recuperación demostrada. Comparar resultados con el banco reservado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si algún paso vuelve al host por necesidad, registrar el último hito alcanzado y la dependencia concreta; conservar el trabajo útil sin declarar cierre nativo.

Entregar `target/self-improvement/tasks/T44/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

