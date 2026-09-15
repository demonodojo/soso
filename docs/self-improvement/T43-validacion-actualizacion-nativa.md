# T43 — Validar y recuperar candidatos construidos en soso

**Hito:** SI-7 · **Tipo:** Integración · **Estado:** pendiente.  
**Dependencias:** [T31](T31-restauracion.md), [T37](T37-mejora-nativa-forja.md), [T42](T42-c-link-imagen.md)

## Objetivo y entrega

Un candidato producido en soso tiene aceptación independiente y recuperación completa demostrada.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [xtask/src/test_update.rs](../../xtask/src/test_update.rs)
- [crates/soso-update-core/src/lib.rs](../../crates/soso-update-core/src/lib.rs)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/validacion.md` y adaptaciones acotadas al runner de pruebas/actualización según fichas nuevas necesarias.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Mapear las comprobaciones host actuales a ejecutables guest o validación en un destino independiente. Enumerar cualquier pérdida de cobertura.
2. Ejecutar suite funcional sobre artefactos construidos por T42; validadores separados del agente y comprobación del hash exacto del candidato.
3. Ensayar instalar candidato y arrancarlo, manteniendo la instancia estable del agente/modelo disponible.
4. Inyectar candidato de kernel y rootfs fallidos; recuperar versión completa usando el procedimiento T31 y volver a ejecutar una petición al modelo.
5. Guardar recibo: fuentes, toolchain, artefactos, pruebas, versión arrancada y versión restaurada. No promover si hay checks requeridos pendientes.

## Comprobación

Pruebas nativas equivalentes del perfil, boot candidato y fallos controlados kernel/rootfs. Cualquier verificador que siga en otra máquina figura con su papel y límites.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si el destino de pruebas requiere una capacidad aún ausente, generar ficha específica; no declarar equivalencia de cobertura sin mapa de pruebas.

Entregar `target/self-improvement/tasks/T43/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

