# soso

Sistema operativo minimalista en Rust: kernel propio bare-metal (x86_64),
filesystem copy-on-write con checksums (sosofs), modelos LLM en un segundo
disco (sosomfs) y acceso por SSH real. Monousuario. Proyecto de aprendizaje
por fases.

Objetivo: arrancar en QEMU, `ssh -p 2222 localhost` con clave ed25519,
usar **sosh** con pipes, redirecciones y directorio de trabajo, y ejecutar
inferencia LLM con `soso-llm`.

## Requisitos

- rustup (el `rust-toolchain.toml` instala solo el nightly fijado; nightly es
  obligatorio porque el build.rs de `bootloader` 0.11 usa `-Zbuild-std`)
- `qemu-system-x86_64` (`sudo apt install qemu-system-x86`)
- OpenSSH client + `ssh-keygen` (para SSH y tests)

## Uso

```sh
cargo xtask build          # compila kernel + genera target/soso-bios.img
cargo xtask run            # build + QEMU q35 con consola serie en stdio
cargo xtask gdb            # como run, congelado en arranque; gdb -ex 'target remote :1234'
cargo xtask test           # integración: FS, boot, TCP, SSH, soso-llm, halt
cargo xtask convert-gguf   # convierte GGUF → layout .som (host)
```

Con soso arrancado (`cargo xtask run`), en otra terminal:

```sh
nc localhost 7777                                              # echo TCP
ssh -tt -i target/soso_test_key -p 2222 soso@localhost         # shell cifrada
```

Salir de QEMU: `Ctrl-A X` (no `Ctrl-C`). Si el puerto 2222 está ocupado:
`pkill qemu-system-x86` antes de volver a arrancar.

## Estructura

- `kernel/` — kernel (no_std, x86_64-unknown-none, fuera del workspace raíz)
- `xtask/` — imagen de disco, userspace, QEMU e integración
- `crates/` — sosofs, sosomfs, soso-abi, soso-llm-core, sosomodel, block-dev
- `tools/` — mkfs-soso, mkfs-sosomfs, mkmodel-soso, convert-gguf, ssh-proto
- `user/` — libsoso, init, sosh, coreutils, soso-llm
- `rootfs/` — árbol embebido en el disco de datos por mkfs-soso

## Características principales

| Área | Detalle |
|------|---------|
| **Shell** | sosh: pipes `\|`, redirecciones `>`, `>>`, `<`, builtins `cd`/`pwd` |
| **Cwd** | Cada proceso tiene directorio de trabajo; rutas relativas respecto a él |
| **FS** | sosofs CoW en virtio-blk0; escritura al `close()` de fd O_WRONLY |
| **Modelos** | sosomfs en virtio-blk1 bajo `/models/`; inferencia con `soso-llm` |
| **Red** | smoltcp, DHCP + fallback 10.0.2.15, SSH sunset en :22 |
| **Syscalls** | spawn (no fork), mmap, pipe, spawn_io, chdir, getcwd (+ GPU) |

## Estado

- [x] Fases 0–10: boot, memoria, kshell, virtio-blk, sosofs R/W, userspace,
      sosh + coreutils, red, SSH con auth ed25519, `cargo xtask test`
- [x] Pipes y redirecciones en sosh (`pipe`, `spawn_io`)
- [x] Cwd por proceso (`chdir`, `getcwd`; builtins `cd`, `pwd`)
- [x] sosomfs + `soso-llm` + conversor GGUF + test E2E de inferencia
- [x] VFS enruta `/models/*` → sosomfs; resto → sosofs
- [x] SSH: sesiones reconectables, salida CRLF en tty remota

**Proyecto completo: 10/10 fases + LLM.** `cargo xtask test` verifica el sistema
de extremo a extremo; `cargo xtask run` arranca soso en QEMU (2 GiB RAM).

Documentación de usuario: [`MANUAL-USUARIO.md`](MANUAL-USUARIO.md).

## Licencia

Copyright (C) 2026 Jose Miguel Díez de la Lastra Jimeno.

El código first-party de soso (kernel, crates, userspace, tools y xtask) se
distribuye bajo **GNU General Public License v2.0 only** (GPL-2.0-only). Ver
[`COPYING`](COPYING) para el texto completo.

Componentes de terceros con licencias propias: [`THIRD_PARTY.md`](THIRD_PARTY.md).
