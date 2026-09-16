# T31 — Demostrar recuperación completa de la instalación

**Hito:** SI-5 · **Tipo:** Integración de recuperación · **Estado:** pendiente.

**Dependencias:** [T30](T30-hardware.md)

## Objetivo y entrega

Sistema anterior vuelve a arrancar y atender una conversación después de fallos kernel y rootfs.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [docs/ESTADO.md](../../docs/ESTADO.md)
- [xtask/src/test_update.rs](../../xtask/src/test_update.rs)
- [crates/soso-update-core/src/lib.rs](../../crates/soso-update-core/src/lib.rs)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/recuperacion.md` y extender un caso aislado en `xtask/src/test_update.rs` solo si falta la prueba requerida.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Leer soso-live. Documentar backup independiente de kernel, rootfs, configuración y modelo; identificar tamaños/hashes y mecanismo concreto de restauración.
2. Ensayar primero sobre imágenes descartables: candidato de kernel que falla y binario rootfs defectuoso. Verificar restauración de ambos, incluido volver a servir el modelo.
3. En hardware usar únicamente el destino identificado y la autorización vigente para operaciones de escritura; si falta acceso, dejar artefactos y procedimiento concreto sin marcar pase.
4. Guardar tiempo de recuperación, versión restaurada y petición de prueba exitosa. No atribuir al rollback de kernel la restauración de rootfs.
5. Mantener la instancia estable accesible durante el ensayo; no habilitar despliegue automático hasta acreditar este cierre.

## Comprobación

`cargo test -p soso-update-core --features std --tests`; `cargo xtask test-update` cuando se modifica el caso; evidencia física de restauración completa por separado.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Rollback rootfs ausente no se resuelve fingiendo que el shim lo cubre; restauración por imagen es válida si está realmente probada.

Entregar `target/self-improvement/tasks/T31/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
