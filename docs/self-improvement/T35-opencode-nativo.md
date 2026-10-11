# T35 — Agente nativo en modo no interactivo (antes: OpenCode nativo)

**Hito:** SI-6 · **Tipo:** Integración condicionada al port · **Estado:** en curso (2026-10-11): redefinida por la decisión **D10** (agente nativo en Rust en vez de portar OpenCode/Bun; ver [DECISIONES.md](DECISIONES.md) y [seguimiento/T35.md](seguimiento/T35.md)).

**Dependencias:** [T19](T19-qemu-e2e.md), [T34](T34-tickets-port.md)

## Objetivo y entrega

**(D10)** Un **agente nativo en Rust** como proceso soso, en modo no interactivo, realiza lectura/búsqueda/edición con herramientas, ejecuta comprobaciones, habla con el modelo guest por la API de `soso-llm` y persiste su sesión. El build del sistema puede seguir remoto. Texto original de la ficha (port de OpenCode): «OpenCode como proceso soso realiza lectura/edición y persiste sesión»; aplazado.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [user/init/src/main.rs](../../user/init/src/main.rs)
- [user/soso-std-test/src/main.rs](../../user/soso-std-test/src/main.rs)
- [xtask/src/test.rs](../../xtask/src/test.rs)

## Archivos que se pueden cambiar

Módulo portable `agente` en `crates/soso-improve-core` (no_std + alloc), adaptadores en `tools/soso-improve` (host) y `user/soso-improve` (guest); aserciones compartidas con T49 y caso de laboratorio en `xtask/src/test_agent_native.rs`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Entrada: `soso-improve` guest compilado con el módulo `agente`, imagen con `soso-llm` y el modelo de trabajo (D9); verificar ELF/ABI.
2. Arrancar QEMU limpio con ese binario; ejecutar --version y después run contra el modelo guest. Registrar PID, versión, kernel y modelo para acreditar dónde corren.
3. Ejecutar lectura y edición de un archivo temporal con herramientas del agente; validar contenido desde un proceso separado.
4. Terminar y arrancar otra vez el agente; recuperar sesión almacenada y comprobar una conversación posterior sin pérdida.
5. Mantener TUI fuera del caso; registrar funciones desactivadas de forma explícita y reproducible.

## Comprobación

Nuevo caso `cargo xtask test-agent-native` tras registrarlo. Afirmar ELF ejecutado en guest, cambios reales de archivo, respuesta del modelo y recuperación de sesión.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Un CLI puente o un agente en Linux conectado a SSH no satisface este cierre.

Entregar `target/self-improvement/tasks/T35/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Empaquetado reproducible mediante receta Rust portable; runtime, proveedor y utilidades disponibles offline. Repetir las aserciones desde T49; xtask es solo lanzador de laboratorio.

Validación nativa: **pendiente**. Condiciones adicionales: [T47](T47-procesos-nativos.md), [T49](T49-pruebas-guest.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
