# Sal en soso: port CPU de usuario

Objetivo: escribir utilidades en Sal y completar **editar → compilar → ejecutar**
dentro de soso, sin red ni Forja. El kernel sigue en Rust; no hay perfil Sal
freestanding para módulos del kernel ni autohospedaje del compilador escrito en Sal.

## Implementación y procedencia

Los cambios del compilador y del runtime C están en el repositorio hermano
[Sal](https://github.com/demonodojo/sal), no en una copia dentro de soso.
La revisión integrada es `ed3bc6dc47d0e29dcde5ddc32f10e3635afdfaa6`.
[`config/sal-source.lock`](../config/sal-source.lock) fija esa revisión (campo
`base`) y el SHA-256 del contenido de `Cargo.toml`, `Cargo.lock`, `build.rs`,
`src/` y `runtime/`. Se verifica antes y después del build.

El port ya está registrado en el commit `0f900ff` de Sal, disponible en
`origin/main` al comprobarlo el 25 de octubre. `ed3bc6d` añade un cambio en
`examples/infer/tiny_prefill.sal`; está en el checkout local y todavía no en
`origin/main`. El compilador y el runtime tienen el mismo hash de contenido
que la integración anterior sobre `dbfe6b3`; ya no dependen de cambios sin
commit en Sal.

| Etapa | Implementación |
|---|---|
| SAL-1: destino | `--target soso`, `-o`, triple x86-64, herramientas configurables, identidad de compilador/herramientas/SDK en caché |
| SAL-2: runtime | C común y adaptador `user/sal-runtime` sobre libsoso; arranque `main(i64, argv) -> i64`, archivos, consola, entorno, procesos y cálculo CPU |
| SAL-3: utilidad y SDK | `sal-contar`, fuentes y ejemplos, SDK estático en `/lib/sal`, empaquetado explícito por xtask |
| SAL-4: compilación nativa | Compilador Rust con std de soso, backend LLVM 19 `sal-llc` y enlazador nativo `wild-soso` |

El backend usa las bibliotecas LLVM y los adaptadores del port existente de
Clang/LLVM (`tests/self-improvement/native/llvm/shims.rs`). Expone únicamente
IR LLVM → objeto ELF; no necesita el frontend C completo. `wild-soso` nativo
fija su propia distribución en `0x400000`; el enlace cruzado usa `user/link.ld`.
La std actual necesita las cabeceras de sección mapeadas: el compilador Rust
se enlaza con Wild del host, forzando las bibliotecas estáticas. Se usa
`panic=abort`; `__register_frame` tiene el mismo stub que en el enlazador nativo.

El runtime v1 es monohilo. Se rechazan instrumentación, GPU y TPU, incluso en
imports. `exec_capture` acepta argumentos separados por espacios, sin sintaxis
de shell. `sal run archivo.sal -- "dos palabras"` conserva los argumentos y
el código de salida. Las utilidades conservan las representaciones de cadenas
y colecciones de Sal. Los códigos de proceso tienen los 8 bits del ABI de soso.

## Construir desde Linux

Requisitos: toolchain habitual de soso, Clang, Python 3 y, para el compilador
nativo, LLVM 19 con cabeceras/bibliotecas estáticas, Wild en PATH y la toolchain
Rust/std de soso preparada según [SELF-HOSTING.md](SELF-HOSTING.md).

```sh
cargo xtask sal-build --cross-only  # utilidad y SDK
cargo xtask sal-build               # añade compilador y backend nativos
cargo xtask test-sal                # construye y prueba offline en QEMU
```

`SOSO_SAL_SOURCE` permite elegir otro checkout de Sal; por defecto es `../sal`.
`SOSO_RUST_VENDOR` elige la toolchain (por defecto `~/.cache/soso-rust-vendor`).
`SAL_LLVM_CONFIG` elige LLVM 19 (por defecto `/usr/lib/llvm-19/bin/llvm-config`).
`SAL_HOST_CC` permite elegir el Clang que construye el runtime C.
Si `/tmp` tiene poca cuota, usa `TMPDIR="$PWD/target/sal/tmp"` tras crear ese
directorio. No hacen falta privilegios ni cambios en discos físicos.

Para incorporar una revisión nueva de Sal, revisar el diff y actualizar el
pin explícitamente con `python3 scripts/sal-source.py --update`; después,
reconstruir con `cargo xtask sal-build`. Los builds normales de soso empaquetan
los artefactos preparados y comprueban el pin. No descargan ni recompilan Sal
implícitamente. `--cross-only` no certifica un compilador nativo de otra revisión.

Ejemplo de compilación cruzada con el SDK generado:

```sh
SAL_SDK="$PWD/target/sal/sdk" \
SAL_LD="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld" \
../sal/target/release/sal build tests/sal/hello.sal --target soso -o target/sal/hello
```

`SAL_CC`, `SAL_LD` y `SAL_SDK` configuran las herramientas del compilador.
El SDK contiene `libsal-soso.a` (arranque y adaptador), `runtime.o`, `kernels.o`,
`sal_runtime.h`, `link.ld` y un manifiesto con hashes. El destino predeterminado
es Linux en Linux y soso dentro de soso. Los objetos de ambos destinos no
comparten clave de caché. La identidad incluye también los bytes de las
herramientas para detectar reconstrucciones con la misma cadena de versión.

## Uso y archivos instalados

Ver [manual de usuario](../MANUAL-USUARIO.md#sal--programar-utilidades).
Se instalan `/bin/sal`, `/bin/sal-contar`, `/lib/sal/bin/sal-llc`, el SDK en
`/lib/sal` y los fuentes de ejemplo en `/usr/share/sal`.

`sal-contar` cuenta bytes y caracteres LF, no líneas lógicas: una última línea
sin LF no aumenta el segundo total. Rechaza NUL y UTF-8 inválido (incluidos
overlong, surrogates, secuencias truncadas y valores mayores de U+10FFFF).

## Validación y seguimiento

`tests/sal/probar.py` crea su propio rootfs e imagen; QEMU lleva `-nic none`.
Empieza sin objetos de los programas ni caché. Los únicos objetos precargados
son el SDK de enlace, no sustitutos de compilaciones de los fuentes de prueba.
Comprueba códigos de salida reales mediante `sal-sonda`, retorno 42, argumentos
con espacios, imports, cadenas UTF-8, colecciones, archivos, entorno, procesos,
errores semánticos/de lectura y recompilación 7 → 9. Compara el comportamiento
de las utilidades cruzadas y nativas. Conserva `target/sal/test/serial.log`.

`scripts/sal-elf.py` comprueba EXEC x86-64, segmentos desde `0x400000`, entrada
ejecutable, símbolos globales resueltos y ausencia de intérprete/sección dinámica.
Las regresiones específicas del compilador están en `tests/soso_target.rs` y
`tests/cli_target.rs` de Sal.

Durante la validación se corrigió `dup2` de tuberías: faltaban referencias de
lectores/escritores, lo que permitía un EOF prematuro al cerrar un duplicado.
La regresión de `user/init` conserva un escritor duplicado y comprueba que la
lectura espera datos hasta cerrar el último escritor.

### Evidencia del 24 de octubre de 2026

Base de soso al terminar la integración: `46161c2f8`, más los cambios locales
del port. El checkout compartido contiene también trabajo de GPU/inferencia;
se conserva y no se atribuye al port de Sal.

| Comprobación | Resultado | Evidencia local |
|---|---|---|
| `cargo xtask sal-build` | OK, código 0; cinco ELF estáticos verificados | `target/sal/build.log` |
| `cargo xtask test-sal` | OK, código 0; ciclo nativo completo offline, incluidos procesos y errores | `target/sal/test-sal.log`, `target/sal/test/serial.log` |
| Sal: `cli_target`, `soso_target`, `toolchain`, `run_hello`, `semantics` | 94 pruebas OK | `target/sal/focused-tests.log`, `target/sal/soso-target-final.log` |
| Suite completa de Sal, `--no-fail-fast -- --test-threads=1` | Falla en `exec_dyn` y `selfhost_ir` | `target/sal/sal-tests.log` |
| `cargo xtask check` | Dos lotes host fallan; builds y hostchecks de drivers OK | `target/sal/xtask-check.log` |
| `cargo xtask test` | Código 1: falla solo `reclaim` con 96 MiB; host, sistema y LLM normales OK | `target/sal/xtask-test.log`, `target/test-reclaim-serial.log` |
| `cargo xtask test -- --guest sys --only init` | OK, código 0; incluye duplicación de lectores y escritores de tubería | `target/sal/init-test.log` |

Los fallos de Sal afectan a las dimensiones dinámicas del trabajo concurrente
de inferencia: un operando de `matmul` queda nulo, aparece `%x_d0` sin definir
y diverge el IR entre bootstrap y selfhost. `exec_dyn` sí pasa en la revisión
base sin cambios (`target/sal/baseline-test.log`). No se han corregido aquí los
cambios ajenos de IR/autohospedaje ni se han desactivado sus pruebas.

En `xtask check` fallan `soso-llm-api::cuda_capa` (diferencia numérica CPU/GPU)
y `soso-improve::receta` (el script y la receta producen hashes diferentes en
diez archivos de std). El primer intento además agotó la cuota de `/tmp`;
se repitió con `TMPDIR` en el proyecto para obtener el resultado completo.

El shard `reclaim` no llega a soso: el cargador UEFI termina con
`OUT_OF_RESOURCES` al arrancar el kernel actual con 96 MiB. El arnés agota
sus 300 segundos esperando la shell. No se ha aumentado la memoria del banco
ni se ha omitido ese fallo para presentar el resultado como verde.

SAL-1–SAL-4 están implementadas y el objetivo funcional nativo está comprobado.
El cierre global de regresiones sigue pendiente de resolver los fallos indicados.
La situación del pin de Sal se actualiza en el apartado siguiente.
La validación de QEMU no acredita hardware físico.

### Actualización del 25 de octubre de 2026

Se revisaron `0f900ff` y `ed3bc6d` y se consultó `origin/main` mediante
`git fetch origin`. El checkout de Sal está limpio; `main` lleva un commit
local por delante del remoto. Se actualiza el pin de `dbfe6b3` + contenido
local a `ed3bc6d`, que registra ese mismo compilador/runtime y el ejemplo
actualizado. El hash de las entradas de construcción sigue siendo
`326d82e4c3bf27b7ad482d4448bef3c4844c51a5a225f184d4219e749aaf72d0`.

Validación de la actualización (ambos comandos con código de salida 0):

- `cargo xtask test-sal`: reconstruye e instala SDK, utilidad, compilador y
  backend; verifica los cinco ELF estáticos y pasa el ciclo nativo offline de
  compilación, ejecución, errores y recompilación tras editar.
- `cargo test --manifest-path ../sal/Cargo.toml --test bench_infer --test soso_target
  --test cli_target --test toolchain --test run_hello --test semantics -- --test-threads=1`:
  96 pruebas OK, incluidas las del ejemplo actualizado y la exportación tiny.

Los recibos de `target/sal`, `target/sal/native` y `rootfs/lib/sal` coinciden
con el pin nuevo. Evidencia conservada en `target/sal/update-2026-10-25/`:
`test-sal.log`, `sal-tests.log`, `serial.log` y `source.lock`.
Los resultados generales del día anterior se conservan como evidencia histórica;
esta actualización no los da por resueltos ni repite las suites globales, ya
que las entradas del compilador/runtime son idénticas a las validadas entonces.
