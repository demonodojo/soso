# T26 — Validar candidatos y promover solo los aceptados

**Hito:** SI-4 · **Tipo:** Implementación portable con adaptadores host/guest · **Estado:** pendiente.

**Dependencias:** [T24](T24-checkout.md), [T25](T25-ejecutor.md)

## Objetivo y entrega

La aprobación del candidato depende de evidencia externa y no de texto del modelo; no se pierde ninguna base anterior.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [xtask/src/check.rs](../../xtask/src/check.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)
- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)
- [user/soso-improve/src/main.rs](../../user/soso-improve/src/main.rs)

## Archivos que se pueden cambiar

Crear `crates/soso-improve-core/src/validate.rs` y pruebas compartidas; conectar `validate` en ambos adaptadores. Definir un receptor/ejecutor guest en la instancia validadora: paquete con run id/base/delta hashes, lista de comprobaciones autorizadas y respuesta con códigos/hashes. Si requiere implementar transporte adicional, derivar una ficha acotada sobre T48.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Reconstruir candidato desde captura+delta T50 en instancia distinta de la sesión del agente. Comprobar rutas editables y hashes del paquete.
2. Ejecutar verificadores independientes y comandos requeridos desde una especificación fuera del alcance del candidato. En la campaña nativa también el validador se ejecuta en soso. Recoger cada código y señal, sin buscar palabras como OK para decidir.
3. Si cambia un test de regresión, ejecutarlo sobre la base para demostrar que detecta el defecto; para rendimiento comparar bajo el mismo perfil.
4. Solo con todas las comprobaciones requeridas en verde escribir accepted y actualizar una referencia de base de la campaña mediante operación durable T46 condicionada al hash anterior (identidad de contenido, sin rama Git obligatoria).
5. Ante fallo o cambio concurrente de base, conservar candidato y no promover. Producir diff e informe revisables; no publicar ni desplegar.

## Comprobación

`cargo test -p soso-improve-core --test validate`. Falso OK con exit 1, test eliminado, rutas fuera de alcance, verificador alterado, fallo QEMU y dos promociones simultáneas. Probar que solo un candidato aceptado cambia la referencia.

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

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Validador y casos reservados residen fuera de la instancia modificable. Definir envío de paquetes por hash, respuesta correlacionada y autoridad de promoción en un receptor soso; T43 aporta arranque y recuperación. No basta guardar tests fuera del checkout.

Validación nativa: **pendiente**. Condiciones adicionales: [T41](T41-cargo-offline.md), [T43](T43-validacion-actualizacion-nativa.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
