# Toolchain nativa: qué hay de verdad

Inventario de [T38](../T38-toolchain-inventario.md). Los datos vivos están en
[toolchain-lock.json](toolchain-lock.json); esto explica qué significan.

**La regla de la ficha**: ninguna herramienta se marca nativa por su nombre ni
por su `--version`. Aquí cada una lleva una **prueba de resultado** — qué
produjo cuando se ejecutó —, y tres de ellas cambiaron de estado al aplicarla.

## El resumen en una tabla

| Herramienta | Dónde corre | Estado |
|---|---|---|
| rustc, cargo (nightly-2026-07-01) | host | real |
| `rust-lld` (dentro del sysroot) | host | real, y es lo que enlaza lo que hoy funciona |
| clang 18.1.3 | host | real (N-005 lo acredita) |
| fork rust-soso (libstd/PAL) | host | clonado y parcheado; **libstd para soso sin construir** |
| `sosoas` | host | no es un ensamblador (sólo `.byte`); su ELF **ya es válido** desde [T67](../T67-sosoas-elf-desplazado.md), pero sin símbolos |
| `wild-soso` | host | nombres alineados desde [T68](../T68-wild-soso-nombre.md); **sigue sin enlazar**: `wild` no está instalado |
| `soso-rustc` | guest | stub, y lo dice |
| mkfs/bootloader | host | real |
| git | host | real; sin sustituto nativo |

## Lo que la regla destapó

### `sosoas` no ensambla

Su encabezado dice «Ensamblador GAS x86_64 de subconjunto». Con un `.s` de
verdad:

    $ sosoas -o real.o real.s      # movl %edi, %eax / addl %esi, %eax / ret
    sosoas: real.s: sin bytes (.byte)
    rc=1

Sólo transcribe directivas `.byte` en hexadecimal. Eso es defendible como
punto de partida — lo que no es defendible es llamarlo ensamblador, porque el
nombre es lo único que mira quien lo da por hecho.

El objeto que **sí** producía tenía además la cabecera ELF desplazada dos
bytes, y los section headers ocho. **Arreglado en
[T67](../T67-sosoas-elf-desplazado.md)**: hoy `readelf` lo valida y `objdump`
desensambla los bytes. Lo que sigue faltando es la tabla de símbolos, así que
`.globl` se ignora y el objeto no sirve para enlazar.

### `wild-soso` no puede ser el enlazador de nadie

Su `Cargo.toml` declara `[[bin]] name = "wild"`. Mientras tanto:

- `targets/x86_64-unknown-soso.json` dice `"linker": "wild-soso"`;
- el `config.toml` del bootstrap apunta a `…/wild-soso/target/release/wild-soso`.

Ninguno de esos dos ficheros existía jamás, y el envoltorio llama a
`Command::new("wild")`, así que si su directorio entraba en el PATH se llamaba
a sí mismo. **Arreglado en [T68](../T68-wild-soso-nombre.md)**, junto con dos
rutas más del mismo tipo: el bootstrap y `xtask` apuntaban a
`tools/*/target/release`, que no existen porque los dos crates son miembros del
workspace, y el bootstrap compilaba sin `--release`.

Además **`wild` no está instalado** en este host, así que aunque los nombres
cuadraran, ese target no podría enlazar hoy.

### El sysroot de soso no existe

`~/.cache/soso-rust-vendor/build-soso/` contiene sólo
`x86_64-unknown-linux-gnu`. No hay directorio de `x86_64-unknown-soso`: el
`x.py build library/std --target x86_64-unknown-soso` no ha llegado a
completarse.

## El paso 3 de la ficha, contestado por los propios ficheros

> Comprobar que el bootstrap actual configura host Linux: compilar `--target
> soso` no genera automáticamente un rustc que ejecute en soso.

No hace falta ejecutar nada: `targets/x86_64-unknown-soso.json` declara
`"host_tools": false`, y el `config.toml` que genera el bootstrap fija
`host = ["x86_64-unknown-linux-gnu"]`. Con esa especificación, un rustc que
corra **en** soso no sale de ahí ni por accidente.

Conviene tenerlo escrito porque es justo la confusión que la ficha teme: un
target que se llama `x86_64-unknown-soso` invita a pensar que produce
herramientas para soso, y lo que produce son binarios **para** soso hechos
**en** Linux.

## Las dos rutas, que no son la misma

Hay dos targets y es fácil confundirlos:

| | `user/x86_64-soso-user.json` | `targets/x86_64-unknown-soso.json` |
|---|---|---|
| Para qué | lo que se compila hoy | ruta A de autohospedaje |
| `std` | no (`no_std + alloc`) | sí |
| Enlazador | `rust-lld` (existe) | `wild-soso` (no existe) |
| Estado | **funciona**, lo acredita la suite | sin libstd construida |

Todo lo que arranca en el guest hoy sale del primero. El segundo es la ruta
que T39–T42 tienen que levantar.

## Sondas de resultado (paso 4)

Definidas aquí para que cada hito posterior tenga un criterio que no sea «la
herramienta está instalada». Las dos primeras ya se han ejecutado y **fallan**,
que es información:

| Sonda | Qué acredita | Hoy |
|---|---|---|
| **emitir objeto** | `sosoas` produce un `.o` que `readelf -h` valida y `nm` lee | **pasa** desde T67 — sin símbolos, que es un límite aparte |
| **enlazar ejecutable** | el enlazador del target produce un ELF que el guest arranca | **falla**: `wild` no está instalado (los nombres ya cuadran, T68) |
| **build offline con Cargo** | `cargo build --offline` para el target soso, sin red | pendiente: necesita libstd |
| **macro procedural / build.rs** | un build script y una proc macro se ejecutan durante la compilación | pendiente |
| **C + asm** | un fichero C y uno de ensamblador se compilan y enlazan juntos | la mitad de C **pasa** (N-005); la de asm necesita símbolos en `sosoas` y un enlazador |
| **creación de imagen** | la imagen resultante arranca | **pasa** (`cargo xtask test`) |

## Recursos, medidos y no medidos

- El clon del fork ocupa lo que ocupa `~/.cache/soso-rust-vendor`; el build de
  `library/std` para un target nuevo **no se ha medido** porque la ficha
  prohíbe arrancar un stage2 dentro de esta inspección. Queda como incógnita,
  no como cero.
- Lo que sí está medido es el camino que funciona: la suite completa compila y
  arranca en los tiempos que registra `cargo xtask test`.

## Perfil mínimo inicial

`rustc-host`, `rust-lld`, `clang`, `mkfs-soso` y `git`. Es exactamente lo que
hace falta para lo que hoy arranca, y no incluye `sosoas` ni `wild-soso`:
ninguno participa en el camino que funciona.

El perfil **lxdde** añade la compilación C del árbol de drivers y se inventaría
aparte cuando esa rama lo pida; mezclarlo aquí escondería que el mínimo es
pequeño.
