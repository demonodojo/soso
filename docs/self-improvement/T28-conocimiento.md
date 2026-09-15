# T28 — Guardar y recuperar aprendizaje verificado

**Hito:** SI-4 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T26](T26-validador.md)

## Objetivo y entrega

La siguiente tarea recibe solo notas pertinentes con procedencia y estado de vigencia.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [docs/ESTADO.md](../../docs/ESTADO.md)

## Archivos que se pueden cambiar

Crear `tools/soso-improve/src/knowledge.rs`, `tests/knowledge.rs` y formato de notas en `docs/self-improvement/conocimiento/`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Definir notas breves: problema, solución, archivos/símbolos, base/candidato, pruebas y enlaces a evidencia. Separar hecho verificado, hipótesis y limitación.
2. Generar candidatas de nota a partir de intentos; aceptar como hecho solo si el validador confirma la evidencia citada.
3. Recuperar por rutas/palabras clave de la siguiente TaskSpec, con orden determinista y presupuesto de bytes/tokens configurable.
4. Invalidar o marcar desactualizada una nota cuando cambian sus archivos/base relevantes; mantener el historial.
5. Prohibir incluir verificadores reservados, secretos y transcripts completos en el paquete de contexto.

## Comprobación

`cargo test -p soso-improve --test knowledge`. Nota de intento fallido, evidencia ausente, selección por ruta, truncamiento, orden estable y cambio de base.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No añadir embeddings, base vectorial ni entrenamiento de pesos en esta entrega.

Entregar `target/self-improvement/tasks/T28/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

