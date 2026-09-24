# Inventario de dependencias de OpenCode para `opencode run` (T32)

Revisión fijada en [`opencode-lock.json`](opencode-lock.json): **v1.18.32**,
commit `545f51d26cc39a907d2867492d498d9607ea5fa4`, licencia MIT. Checkout en
`target/self-improvement/opencode/src`, fuera del árbol fuente de soso.

Este documento es **inventario, no plan de port**. No se intenta aquí portar
Bun ni escribir una capa POSIX: eso es T35, y llegar a esta lista es lo que
permite decidir si tiene sentido intentarlo.

Las rutas de esta página apuntan al checkout fijado. Las capacidades de soso
apuntan a un símbolo real o a una comprobación pendiente de
[T33](../T33-sondas-abi.md); **ninguna casilla se marca por el nombre de una
función**.

## Lo primero, porque cambia el resto

Dos cosas que conviene saber antes de leer la matriz:

1. **El repositorio se movió.** `CONTRATO.md` cita `opencode.ai`, y el código
   vivía en `sst/opencode`; hoy es `anomalyco/opencode` y la URL antigua
   redirige. Se fija la nueva. Un plan que clone la antigua sin `-L` fallará.
2. **`opencode run` no es un modo sin TUI.** La ficha lo daba por supuesto y en
   esta revisión no lo es: `packages/opencode/src/cli/cmd/run.ts` importa
   `@opentui/core`, `@opentui/keymap` y `@opentui/solid` en el módulo, y la
   ejecución no interactiva es una **rama** dentro de él
   (`if (interactive && !process.stdout.isTTY)`, `run.ts:319`; la entrada por
   tubería en `run.ts:416`). El terminal UI se enlaza aunque no se use. Separar
   «run» de «TUI» exige tocar el código de OpenCode, no sólo elegir un comando.

## Runtime

| Pieza | Revisión | Qué implica |
|---|---|---|
| **Bun** | `1.3.14` (`package.json`, campo `packageManager`) | Es el runtime, no un empaquetador. Trae **JavaScriptCore** (no V8), su propio `fetch` con TLS, `bun:sqlite`, `Bun.spawn` y su bucle de eventos. Todo lo de abajo se apoya en él. |

Portar OpenCode **es** portar Bun. No hay build a binario nativo en el
repositorio: el `install` publicado descarga artefactos por plataforma.

## Matriz dependencia → uso → capacidad del SO → estado en soso

Estado: **soportado** (hay símbolo y semántica comprobada), **parcial**,
**ausente**, **por comprobar** (T33).

### Núcleo de `opencode run`

| Dependencia | Uso | Capacidad del SO | soso |
|---|---|---|---|
| Bun / JavaScriptCore | ejecutar el programa | JIT: `mmap` con **W+X**, señales, TLS de hilo | **ausente**: `SYS_MPROTECT` existe (`soso-abi`), pero que permita W+X y que JSC funcione sin señales POSIX es justo lo que hay que medir → T33 |
| `effect` 4.0.0-beta.83 | efectos, concurrencia, bucle | temporizadores, microtareas | **por comprobar**: puro JS, depende del runtime |
| `@effect/sql-sqlite-bun`, `drizzle-orm` | sesiones, historial | **SQLite** dentro de Bun: `pread`/`pwrite`, `fsync`, **bloqueo de ficheros**, `mmap` | **parcial**: hay `SYS_PWRITE`, `SYS_SEEK`, `SYS_READ`, `SYS_FSYNC` y `SYS_MMAP` con fd (`kernel/src/task/syscall.rs:1491`, respaldo por inodo). **No hay `pread` ni `flock`/`fcntl`** → T33 |
| `cross-spawn` 7.0.6, `which` | lanzar herramientas | `fork`+`exec`, `PATH`, códigos de salida | **parcial**: soso tiene `SYS_SPAWN`/`SYS_SPAWN_IO` (spawn directo) y **no** `fork`/`exec`. Semántica distinta: sin `fork` no hay `posix_spawn` con acciones arbitrarias → T33 |
| `ai` 6.0.168 + `@ai-sdk/*` | hablar con el modelo | HTTPS cliente, SSE | **soportado en otro sitio**: soso tiene cliente HTTP/TLS propio (`crates/soso-http`, usado por `soso-update`), pero **no** expuesto a un runtime JS. El `fetch` que usaría es el de Bun |
| `zod` 4.1.8, `remeda`, `diff`, `semver`, `minimatch`, `ignore`, `glob` | validación y utilidades | ninguna | **soportado**: JS puro |
| `xdg-basedir`, `gray-matter`, `jsonc-parser` | configuración | `getenv`, lectura de ficheros | **soportado**: `SYS_GETENV`, `SYS_OPEN`/`SYS_READ` |

### Herramientas del agente (`packages/core/src/tool/`)

Son las que el candidato usa de verdad; sin ellas el agente no edita nada.

| Herramienta | Depende de | soso |
|---|---|---|
| `read`, `write`, `edit`, `apply-patch`, `glob` | ficheros | **soportado**: `SYS_OPEN`, `SYS_READ`, `SYS_WRITE`, `SYS_GETDENTS`, `SYS_STAT`, `SYS_UNLINK`, `SYS_MKDIR`, `SYS_RENAME` |
| `grep` | **ripgrep 15.1.0**, binario externo que se **descarga en ejecución** (`packages/core/src/ripgrep/binary.ts:105`) | **ausente**: binario ajeno x86-64 Linux; ni está en la imagen ni hay quien lo compile. Sustituto: el `grep` de soso, con semántica distinta |
| `bash` | `/bin/bash` y `bash -c` | **ausente**: soso tiene `sosh`, que no es bash y no pretende serlo. 35 apariciones de `"bash"` en el código |
| `webfetch`, `websearch` | HTTPS saliente | **parcial**: hay pila TLS, no expuesta a JS |
| `question`, `todowrite`, `skill` | ninguna | **soportado** |

Binarios externos citados en `packages/core/src` y `packages/opencode/src`
(conteo de literales): `git` 48, `bash` 35, `npm` 16, `curl` 11, `bun` 9,
`grep` 8, `which` 3, `rg` 3, `sh` 1, `node` 1. **Ninguno existe en soso** salvo
un `grep` propio.

### Nativos: lo que no es JavaScript

Cada uno es un `.node` compilado **por plataforma**, cargado con `dlopen`.

| Paquete | Versión | Para qué | soso |
|---|---|---|---|
| `@lydell/node-pty`, `bun-pty` | 1.2.0-beta.12 / 0.4.8 | terminal del agente | **ausente**: no hay pty, `termios` ni `ioctl` en los 93 syscalls |
| `@parcel/watcher` | 2.5.1 | vigilar ficheros | **ausente**: no hay `inotify` ni equivalente. El lockfile trae un `.node` por plataforma (`darwin-arm64`, `linux-x64-glibc`, `linux-x64-musl`…), ninguno para soso |
| `tree-sitter-bash` | 0.25.0 | parsear bash | **ausente** |
| `tree-sitter-powershell` | 0.25.10 | parsear PowerShell | **ausente** |
| `web-tree-sitter` | 0.25.10 | igual, en **WASM** | **por comprobar**: WASM lo ejecuta JSC, así que depende sólo de Bun |
| `@silvia-odwyer/photon-node` | 0.3.4 | imágenes | **ausente**, y **opcional** para `run` |

**El bloqueo de fondo:** soso no tiene `dlopen`. No hay carga dinámica de
código nativo en los 93 syscalls, así que **ningún `.node` es cargable**, ni
recompilándolo. Lo que sea nativo tiene que desaparecer o volverse WASM.

### Sólo TUI, plugins y opcionales — fuera de `opencode run`

`@opentui/core`, `@opentui/keymap`, `@opentui/solid`, `opentui-spinner`,
`solid-js`, `@solid-primitives/*`, `strip-ansi`, `@clack/prompts`,
`fuzzysort`; y de funciones opcionales: `@octokit/*`, `@actions/*`,
`@modelcontextprotocol/sdk`, `bonjour-service` (mDNS, **UDP** — soso sólo tiene
TCP más `SYS_PING` y `SYS_DNS_RESOLVE`), `ws`, `chokidar`, `open`,
`@opentelemetry/*`, `@aws-sdk/credential-providers`, `google-auth-library`,
`@zip.js/zip.js`, `turndown`, `htmlparser2`, `marked`.

**Con la salvedad del principio**: `run.ts` importa `@opentui/*`, así que hoy
«fuera de run» describe el uso, no el enlazado.

## Recursos mínimos

| Qué | Cuánto | Cómo se sabe |
|---|---|---|
| Checkout | **224 MB** con `--depth 1` | medido en el clon fijado |
| `bun.lock` | 879 KB | el propio fichero |
| Dependencias instaladas | **no medido** | no se ejecutó `bun install`: bajaría cientos de MB y ninguna conclusión de esta ficha depende de la cifra. Queda como incógnita, no como cero |
| Compilar | no aplica | no hay build a binario en el repo |
| Ejecutar | **no medido** | hace falta Bun corriendo; es lo que T33 tiene que acotar |

## Incógnitas, como comprobaciones concretas para T33

Cada una es una sonda pequeña, no una opinión:

1. **W+X**: ¿`SYS_MPROTECT` permite una página escribible y ejecutable a la
   vez? Sin eso no hay JIT, y sin JIT no hay JavaScriptCore.
2. **`pread`/`pwrite` posicionales**: hay `SYS_PWRITE`; ¿existe lectura
   posicional o hay que hacer `seek`+`read`? SQLite lo necesita con varios
   descriptores del mismo fichero.
3. **Bloqueo de ficheros**: ¿hay algo equivalente a `flock`/`fcntl`? SQLite lo
   usa para su journal; sin ello, dos procesos corrompen la base.
4. **`spawn` frente a `fork`+`exec`**: ¿qué hereda el hijo exactamente
   (descriptores, cwd, entorno) y qué **no** se puede expresar sin `fork`?
   [T47](../T47-procesos-nativos.md) dejó el adaptador; falta la comparación.
5. **Señales**: soso tiene `SYS_KILL` y los números `SIGINT`, `SIGKILL`,
   `SIGTERM` y `SIGPROBE`, pero **no hay forma de instalar un manejador** —no
   existe `sigaction` ni equivalente—. Un proceso se puede matar; no puede
   *reaccionar*. ¿Qué hace un runtime que espera `SIGCHLD`, `SIGPIPE` y
   `SIGWINCH`? Distinguir «se puede señalar» de «se puede manejar» es
   exactamente el tipo de casilla que no se marca por el nombre.
6. **`mmap` de fichero**: existe con fd; falta comprobar `MAP_SHARED` entre
   procesos y qué pasa al escribir el fichero por debajo.
7. **Carga dinámica**: no hay `dlopen`. La comprobación no es «¿se puede
   añadir?» sino «¿qué parte del inventario sobrevive sin él?».

## Lo que este inventario deja decidido

- **Portar OpenCode tal cual no es viable con el ABI de hoy**, y el motivo no
  es una lista larga de paquetes: son tres cosas concretas — **no hay `dlopen`**
  (mata todos los `.node`), **no hay pty ni `inotify`** (dos dependencias del
  núcleo), y **`opencode run` arrastra el TUI** por importación.
- **La dependencia mayor es Bun**, no los 90 paquetes de npm. Casi todo lo
  demás es JavaScript puro que correría solo si el runtime corre.
- El camino que este inventario sugiere para SI-6 no es «portar OpenCode» sino
  **acotar qué subconjunto del agente hace falta**, que es lo que
  [T35](../T35-opencode-nativo.md) tendrá que decidir con estos datos delante.

Nada de lo anterior cierra SI-6: completar el inventario no es portar nada.
