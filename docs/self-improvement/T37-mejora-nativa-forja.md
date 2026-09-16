# T37 — Cerrar una mejora desde OpenCode nativo con build remoto

**Hito:** SI-6 · **Tipo:** Integración · **Estado:** pendiente.

**Dependencias:** [T22](T22-primera-mejora.md), [T35](T35-opencode-nativo.md), [T36](T36-forja-trazabilidad.md)

## Objetivo y entrega

SI-6 completo para el alcance headless documentado: tarea end-to-end con agente nativo; dependencia de compilación remota explicitada.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)
- [user/soso-forja/src/main.rs](../../user/soso-forja/src/main.rs)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/primera-tarea.md` y caso de integración de tarea en el arnés T35.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Preparar una tarea pequeña de userspace sobre /src/soso, con fuente base conocida y aceptación externa.
2. OpenCode nativo consulta modelo local, edita archivos y llama sync/build de Forja; recibe diagnósticos sin que un humano copie errores al prompt.
3. Validar recibo T36, ejecutar el artefacto en un destino candidato y devolver resultado al agente.
4. Comprobar que la tarea termina y que el propio OpenCode/servidor permanecen operativos.
5. Registrar ubicaciones reales: agente=soso, inferencia=soso, build=host. Repetir la aceptación sobre candidato limpio.

## Comprobación

Caso T35 ampliado más verificadores de la tarea y demo Forja. Prueba negativa con recibo que corresponde a la base anterior.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No cerrar SI-7 ni llamar compilación local a build-local, que hoy copia artefactos.

Entregar `target/self-improvement/tasks/T37/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Registrar etapa mixta explícita: agente/modelo nativos con build remoto. Conservar esta evidencia, pero exigir repetición de build, pruebas e informe sin Forja en T51.

Validación nativa: **pendiente**. Condiciones adicionales: [T51](T51-aceptacion-circuito-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
