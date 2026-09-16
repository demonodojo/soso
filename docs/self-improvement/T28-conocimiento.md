# T28 — Guardar y recuperar aprendizaje verificado

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T26](T26-validador.md)

## Objetivo y entrega

La siguiente tarea recibe solo notas pertinentes con procedencia y estado de vigencia.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [docs/ESTADO.md](../../docs/ESTADO.md)
- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-improve-core/src/knowledge.rs`, pruebas compartidas y órdenes en ambos frontends, más formato de notas en `docs/self-improvement/conocimiento/`.

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

`cargo test -p soso-improve-core --test knowledge`. Nota de intento fallido, evidencia ausente, selección por ruta, truncamiento, orden estable y cambio de base.

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

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Condiciones adicionales: [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
