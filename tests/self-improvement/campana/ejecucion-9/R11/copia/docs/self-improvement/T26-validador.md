# T26 — Validar candidatos y promover solo los aceptados

**Hito:** SI-4 · **Tipo:** Implementación host · **Estado:** pendiente.  
**Dependencias:** [T24](T24-checkout.md), [T25](T25-ejecutor.md)

## Objetivo y entrega

La aprobación del candidato depende de evidencia externa y no de texto del modelo; no se pierde ninguna base anterior.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [xtask/src/check.rs](../../xtask/src/check.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)

## Archivos que se pueden cambiar

Crear `tools/soso-improve/src/validate.rs` y `tests/validate.rs`; conectar `validate`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Reconstruir el candidato desde base+diff en una copia distinta de la sesión del agente. Comprobar rutas editables y hash del parche.
2. Ejecutar verificadores externos y comandos requeridos desde una especificación fuera del alcance del candidato. Recoger cada código y señal, sin buscar palabras como OK para decidir.
3. Si cambia un test de regresión, ejecutarlo sobre la base para demostrar que detecta el defecto; para rendimiento comparar bajo el mismo perfil.
4. Solo con todas las comprobaciones requeridas en verde escribir accepted y actualizar una referencia de base de la campaña mediante operación condicionada al hash anterior.
5. Ante fallo o cambio concurrente de base, conservar candidato y no promover. Producir diff e informe revisables; no publicar ni desplegar.

## Comprobación

`cargo test -p soso-improve --test validate`. Falso OK con exit 1, test eliminado, rutas fuera de alcance, verificador alterado, fallo QEMU y dos promociones simultáneas. Probar que solo un candidato aceptado cambia la referencia.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Los checks completos de C6 son puerta de promoción aunque se hayan pasado tests focalizados durante edición.

Entregar `target/self-improvement/tasks/T26/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

