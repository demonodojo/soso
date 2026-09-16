# T32 — Inventariar dependencias del OpenCode que se quiere portar

**Hito:** SI-6 · **Tipo:** Inspección acotada · **Estado:** pendiente.

**Dependencias:** [T01](T01-base.md)

## Objetivo y entrega

Inventario para el modo sin TUI, con lista finita de capacidades que T33 debe comprobar.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)
- [crates/soso-abi/src/lib.rs](../../crates/soso-abi/src/lib.rs)
- [user/Cargo.toml](../../user/Cargo.toml)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/opencode-lock.json` y `opencode-deps.md`. Checkout externo fijado fuera del árbol fuente de soso.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Obtener el código de la revisión correspondiente a OpenCode evaluado y registrar commit, lockfile, licencia y comandos de build oficiales. No trabajar contra una rama móvil.
2. Inventariar runtime, bibliotecas nativas, base de datos, event loop, TLS, filesystem, subprocess, terminal y herramientas externas a partir de imports/build reales.
3. Separar dependencias necesarias para `opencode run` de TUI, plugins y funciones opcionales. Construir una matriz dependencia→uso→API del SO requerida→fuente.
4. Para cada requisito contrastar implementación en ABI/libsoso/soso-std; marcar soportado, stub, ausente o por comprobar. Un nombre de función no demuestra semántica.
5. Determinar recursos mínimos de compilación/ejecución y artefactos externos requeridos. Registrar incógnitas como comprobaciones concretas para T33.

## Comprobación

Revisión documental: cada dependencia enlaza a un archivo/revisión y cada capacidad soso a un símbolo local o prueba pendiente. Lockfile/commit inequívocos; ningún casillero soportado basado solo en su nombre.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No intentar portar Bun/JavaScriptCore ni escribir una capa POSIX general en esta ficha. Completar inventario no cierra SI-6.

Entregar `target/self-improvement/tasks/T32/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Incluir dependencias transitivas, instalación offline, bibliotecas nativas y cada utilidad invocada por herramientas de OpenCode. Cada dependencia funcional debe ejecutarse en soso o sustituirse con semántica probada.

Validación nativa: **no aplicable a esta entrega de laboratorio/especificación**. Condiciones adicionales: [T35](T35-opencode-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
