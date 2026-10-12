# T14 — Medir calidad y fijar presupuestos antes de usar el agente

**Hito:** SI-0 / SI-1 · **Tipo:** Implementación de evaluación e integración · **Estado:** pendiente.  
**Dependencias:** [T02](T02-banco.md), [T03](T03-perfil-modelo.md), [T13](T13-servidor-host.md)

## Objetivo y entrega

Modelo apto demostrado y presupuestos medidos, o no-go justificado con la siguiente corrección concreta. Un no-go no satisface la dependencia funcional de T16/T22.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C2–C3, C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [crates/soso-llm-core/examples/hostrun.rs](../../crates/soso-llm-core/examples/hostrun.rs)

## Archivos que se pueden cambiar

Crear `scripts/self-improvement/evaluate.py`, `tests/self-improvement/test_evaluate.py` y `docs/self-improvement/evaluacion.md`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Consumir el banco T02 y un endpoint explícito. Medir con reloj monotónico host: primer byte útil, primer token, fin, tokens reales, memoria reportada y fallos.
2. Ejecutar cada caso tres veces con semillas registradas. Separar frío/caliente y corrección/velocidad; incluir todos los intentos en el denominador.
3. Usar verificadores reservados fuera del contexto del modelo. Ejercitar tool request → resultado de herramienta → siguiente petición.
4. Probar contexto representativo de OpenCode y el caso de 8 Ki tokens si cabe. Fijar límite de salida, timeouts finitos y presupuesto por tarea en model-lock.
5. Generar informe go/no-go con 10/10 casos de protocolo y ≥8/10 microtareas por ejecución según el plan, además de variabilidad. Repetir con otro perfil si falla, sin cambiar el banco durante la comparación.

## Comprobación

`python3 -m unittest discover -s tests/self-improvement -p test_evaluate.py`. Backend simulado con éxito, timeout, usage ausente y salida errónea; comprobar que no se cuentan como éxito. Adjuntar campaña real por separado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si el modelo no usa herramientas, no pasar a una demo que ejecute comandos inventados por el adaptador.

Entregar `target/self-improvement/tasks/T14/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

