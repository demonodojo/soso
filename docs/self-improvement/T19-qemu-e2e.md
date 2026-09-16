# T19 — Crear la prueba completa de API dentro de soso

**Hito:** SI-2 · **Tipo:** Implementación de integración QEMU · **Estado:** pendiente.

**Dependencias:** [T16](T16-servicio-guest.md), [T17](T17-admisiones-cancelacion.md), [T18](T18-puertos-qemu.md)

## Objetivo y entrega

Todas las invariantes API pasan en guest con modelo real y no existe fallback host. SI-2 solo cierra con ambos tipos de evidencia.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3–C4, C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [xtask/src/test.rs](../../xtask/src/test.rs)
- [xtask/src/main.rs](../../xtask/src/main.rs)
- [xtask/src/test_distributed.rs](../../xtask/src/test_distributed.rs)

## Archivos que se pueden cambiar

Crear `xtask/src/test_llm_api.rs` y registrar el comando nuevo `cargo xtask test-llm-api` en main.rs; integrar checks host nuevos en check.rs cuando corresponda. Extraer aserciones a módulos portables de pruebas de la API e integrarlas con user/soso-improve mediante T49 para el ensayo nativo.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar arnés con imágenes/copias y puertos propios, proceso QEMU identificado y limpieza por PID de esa ejecución. CPU q35/max y perfil documentado.
2. Modo de transporte con modelo sintético para fallos HTTP; modo real obligatorio para herramientas/calidad. `--model-dir` y perfil explícitos; falta de pesos = resultado bloqueado, nunca pase.
3. Arrancar serve con token temporal provisionado; probar health, models, JSON, SSE, Unicode, cuerpo >1 KiB, límites, autenticación y modelo desconocido.
4. Probar petición cancelada durante prefill, segunda petición 429, health mientras genera y nueva conversación tras desconexión. Comparar uso real y salida sin diagnósticos.
5. Apagar el servidor guest y verificar que falla el cliente. Guardar serial, peticiones/respuestas sin token, perfil y hashes; probar también ask residente con servidor HTTP.

## Comprobación

Nuevo comando: `cargo xtask test-llm-api --model-dir <modelo> --profile <perfil>`. Mantener unit tests sin red externa. Antes del cierre de SI-2 ejecutar C6 completo y T14 contra endpoint guest.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Los éxitos con backend falso o synthetic tiny no cuentan como capacidad de agente; informar por separado.

Entregar `target/self-improvement/tasks/T19/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Mover aserciones reutilizables a módulos portables y ejecutarlas con T49. xtask arranca QEMU solo en laboratorio; T43 aporta destino y control nativos para la campaña final.

Validación nativa: **pendiente**. Condiciones adicionales: [T48](T48-reloj-red.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
