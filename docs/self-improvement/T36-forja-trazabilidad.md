# T36 — Vincular fuentes, build y artefacto de Forja

**Hito:** SI-6 · **Tipo:** Implementación cliente-servidor · **Estado:** **hecha** (2026-09-25).

Resumen: [target/self-improvement/tasks/T36/resultado.md](../../target/self-improvement/tasks/T36/resultado.md).

**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

Fuentes y ELF/pack están unidos por recibo verificable de extremo a extremo.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [tools/soso-forja-server/src/main.rs](../../tools/soso-forja-server/src/main.rs)
- [user/soso-forja/src/main.rs](../../user/soso-forja/src/main.rs)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Editar los dos main.rs de Forja y tests del servidor; introducir módulo puro compartido solo si el contrato se duplica.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Revisar X-Forja-Build-Id y manifiesto hash existentes. Definir recibo versionado: task_id, source_manifest_sha256, build_id, perfil/comandos y hashes de artefactos.
2. Servidor: emitir recibo ligado al conjunto exacto de fuentes aceptado y artefactos generados; excluir restos de builds previos del resultado.
3. Cliente: verificar coincidencia con la petición actual antes de aceptar el pack. Rechazar build-id, source hash o artifact hash distintos.
4. Conservar errores HTTP y fallos de build como fallos; nunca aplicar artefacto obsoleto de /var/forja-out como si proviniera de la petición.
5. Separar sync/build/descarga de apply para que la tarea del agente pueda revisar el resultado sin reiniciar su sistema.

## Comprobación

`cargo test -p soso-forja-server -- --test-threads=1`; build cliente en cwd `user/`; demo `cargo xtask test -- --guest sys --only forja`. Casos pack viejo, source hash distinto, recibo truncado y build fallido.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

No rediseñar OTA ni trasladar el compilador al guest en esta ficha.

Entregar `target/self-improvement/tasks/T36/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Forja es una etapa mixta identificada por plataforma en el manifiesto. El cliente corre en soso; su build Linux no acredita el cierre nativo, que debe repetir T51 sin Forja.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
