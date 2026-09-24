# T32 — Inventariar dependencias del OpenCode que se quiere portar

**Hito:** SI-6 · **Tipo:** Inspección acotada · **Estado:** completada (2026-09-24).

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

## Resultado

Entregados [`native/opencode-lock.json`](native/opencode-lock.json) (revisión
fijada) y [`native/opencode-deps.md`](native/opencode-deps.md) (inventario y
matriz). Checkout en `target/self-improvement/opencode/src`, fuera del árbol
fuente, con `--depth 1 --branch v1.18.32`.

**Revisión fijada:** `anomalyco/opencode` v1.18.32, commit
`545f51d26cc39a907d2867492d498d9607ea5fa4`, MIT, 2026-09-21. No se usa `dev`,
que es la rama por defecto y es móvil.

### Dos hallazgos que cambian el encuadre de esta ficha

1. **El repositorio se movió.** El plan cita `opencode.ai` y el código vivía en
   `sst/opencode`; hoy es `anomalyco/opencode`. La URL vieja redirige, así que
   `git clone` funciona y `curl` sin `-L` no.
2. **`opencode run` no es un modo sin TUI.** Esta ficha lo daba por supuesto.
   `run.ts` importa `@opentui/core`, `@opentui/keymap` y `@opentui/solid` en el
   módulo, y lo no interactivo es una **rama** dentro de él (`run.ts:319`,
   `run.ts:416`). Se puede ejecutar sin terminal; no se puede **enlazar** sin
   él. Separarlos exige tocar el código de OpenCode, y eso es dato para
   [T35](T35-opencode-nativo.md).

### Lo que decide el inventario

El obstáculo no es la longitud de la lista —unos 90 paquetes, casi todos JS
puro que correría solo si el runtime corre—. Son tres cosas:

- **No hay `dlopen`** en los 93 syscalls: **ningún `.node` es cargable**, ni
  recompilándolo. Caen `@lydell/node-pty`, `bun-pty`, `@parcel/watcher` y los
  `tree-sitter` nativos.
- **No hay pty ni `inotify`**, y son dependencias del núcleo, no opcionales.
- **Portar OpenCode es portar Bun**: JavaScriptCore, `fetch` con TLS,
  `bun:sqlite` y su bucle de eventos.

Y sin herramientas el agente no edita nada: `grep` usa **ripgrep 15.1.0**
descargado en ejecución, y `bash` aparece 35 veces en el código.

Las siete incógnitas quedan escritas como sondas concretas para
[T33](T33-sondas-abi.md); la primera decide el resto: si `SYS_MPROTECT` no
permite W+X no hay JIT, y sin JIT no hay JavaScriptCore.

## Comprobación

Revisión documental: cada dependencia enlaza a un archivo/revisión y cada capacidad soso a un símbolo local o prueba pendiente. Lockfile/commit inequívocos; ningún casillero soportado basado solo en su nombre.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Cómo se verificó que ninguna casilla es de oídas.** Las de soso se
comprobaron contra `crates/soso-abi/src/lib.rs` y el kernel: `mmap` respaldado
por fichero **sí** (`kernel/src/task/syscall.rs:1491`); `dlopen`,
`flock`/`fcntl`, `pread` y UDP genérico, **cero apariciones**. Y las señales
son el caso que más fácil se marcaría mal: hay `SYS_KILL` y los números
`SIGINT`/`SIGKILL`/`SIGTERM`/`SIGPROBE`, pero **no hay forma de instalar un
manejador**. Un proceso se puede matar; no puede reaccionar.

**Límite.** `bun install` **no se ejecutó**: los recursos de instalación y
ejecución quedan **no medidos**, no en cero. Completar el inventario no cierra
SI-6.

No intentar portar Bun/JavaScriptCore ni escribir una capa POSIX general en esta ficha. Completar inventario no cierra SI-6.

Entregar `target/self-improvement/tasks/T32/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Incluir dependencias transitivas, instalación offline, bibliotecas nativas y cada utilidad invocada por herramientas de OpenCode. Cada dependencia funcional debe ejecutarse en soso o sustituirse con semántica probada.

Validación nativa: **no aplicable a esta entrega de laboratorio/especificación**. Condiciones adicionales: [T35](T35-opencode-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
