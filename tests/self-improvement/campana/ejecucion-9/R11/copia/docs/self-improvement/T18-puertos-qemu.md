# T18 — Añadir reenvío HTTP configurable sin colisiones

**Hito:** SI-2 · **Tipo:** Implementación xtask · **Estado:** pendiente.  
**Dependencias:** Ninguna; puede iniciarse ahora.

## Objetivo y entrega

Dos VMs pueden tener forwards API distintos; los shards existentes conservan sus puertos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3–C4, C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [xtask/src/main.rs](../../xtask/src/main.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)
- [xtask/src/test_distributed.rs](../../xtask/src/test_distributed.rs)

## Archivos que se pueden cambiar

Crear `xtask/src/llm_ports.rs`; editar main.rs y conexiones de QemuSlot necesarias; tests unitarios en el módulo nuevo.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Extraer construcción pura del argumento slirp y añadir puerto HTTP opcional. Mantener los defaults actuales de SSH/echo cuando no se pide API.
2. Para run aceptar `SOSO_LLM_HOST_PORT`; en tests pasar puerto explícito por instancia, sin heredar una variable global común a todos los shards.
3. Reenviar solo `127.0.0.1:<host> → guest:7422`; validar rango y colisión con puertos de la misma instancia.
4. Aplicar por igual a virtio y e1000e; con NIC VFIO no hay slirp, explicar que el acceso es por IP física.
5. Conservar la firma antigua mediante wrapper o actualizar todos los llamantes con un argumento opcional; registrar qué puertos usa cada VM.

## Comprobación

`cargo test -p xtask llm_ports::`. Casos sin API, API explícita, dos instancias distintas, puerto inválido y VFIO. Inspeccionar argv sin arrancar QEMU; T19 probará la red real.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No introducir un forward fijo en todos los tests ni detener procesos ajenos para liberar puertos.

Entregar `target/self-improvement/tasks/T18/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

