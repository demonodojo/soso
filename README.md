# soso

Sistema operativo minimalista en Rust: kernel propio bare-metal (x86_64),
filesystem propio copy-on-write con checksums (sosofs) y acceso por SSH real.
Monousuario. Proyecto de aprendizaje por fases.

El plan completo (stack, diseño de sosofs, fases 0-10) está en el plan del
proyecto; resumen del objetivo final: arrancar en QEMU, `ssh -p 2222 localhost`
con clave ed25519 y ejecutar binarios ELF propios sobre sosofs.

## Requisitos

- rustup (el `rust-toolchain.toml` instala solo el nightly fijado; nightly es
  obligatorio porque el build.rs de `bootloader` 0.11 usa `-Zbuild-std`)
- `qemu-system-x86_64` (`sudo apt install qemu-system-x86`)

## Uso

```sh
cargo xtask build   # compila kernel + genera target/soso-bios.img
cargo xtask run     # build + QEMU q35 con consola serie en stdio
cargo xtask gdb     # como run, congelado en arranque; gdb -ex 'target remote :1234'
cargo xtask test    # batería de integración (crash-safety, boot, TCP, SSH, halt)
```

Con soso arrancado (`cargo xtask run`), en otra terminal:

```sh
nc localhost 7777                        # echo TCP
ssh -i target/soso_test_key -p 2222 soso@localhost   # shell cifrada
```

Salir de QEMU: `Ctrl-A X`.

## Estructura

- `kernel/` — el kernel (no_std, target x86_64-unknown-none, fuera del workspace raíz)
- `xtask/` — build de la imagen de disco (crate `bootloader`), del userspace y lanzador de QEMU
- `crates/` — sosofs (el FS), soso-abi (ABI de syscalls), block-dev
- `tools/` — mkfs-soso, drive-shell.py y kill-test.py (tests scriptados por serie)
- `user/` — workspace de userspace: libsoso (crt0 + syscalls + mini-libstd),
  init, sosh y coreutils
- `tools/ssh-proto/` — prototipo host de sunset (fase 9), aislado del workspace

## Estado

- [x] Fase 0: boot + consola serie
- [x] Fase 1: memoria e interrupciones (GDT/TSS, IDT, PIC+PIT 100 Hz, frames, heap talc)
- [x] Fase 2: kernel-shell por serie (`help uptime mem pf panic halt`)
- [x] Fase 3: PCI (ECAM) + virtio-blk (`blk blkread blkwrite`, persistente entre arranques)
- [x] Fase 4: sosofs solo-lectura + mkfs-soso (`ls cat stat`; checksums verificados en todo)
- [x] Fase 5: sosofs escritura CoW (`write mkdir rm df`; transacciones con commit
      atómico A/B, bitmap dual, crash-injection host + kill -9 de QEMU en mitad
      de escrituras)
- [x] Fase 6: userspace (ring 3 vía syscall/sysret, 14 syscalls, ELF a 0x400000,
      round-robin preemptivo, PML4 por proceso, libsoso + /bin/init; un page
      fault de usuario mata al proceso, no al kernel)
- [x] Fase 7: sosh + coreutils (ls cat echo mkdir rm hexdump) como ELFs; init
      relanza la shell si muere; `exit` limpio → kernel-shell de emergencia;
      `/bin/init test` = suite de regresión de las 14 syscalls
- [x] Fase 8: virtio-net + smoltcp (IP 10.0.2.15, echo TCP en :7; `nc localhost
      7777` responde; pool de sockets en escucha; la red se atiende desde el
      scheduler y el tick de timer — sin interrupciones de NIC)
- [x] Fase 9: SSH real con sunset (`ssh -p 2222 localhost` abre una sosh cifrada;
      curve25519 + ed25519 + chacha20-poly1305; canal de shell sobre smoltcp;
      host key ed25519 generada al arranque). Requiere QEMU `-cpu max` (RDRAND).
- [x] Fase 10: auth SOLO por clave pública (`/etc/authorized_key` inyectada por
      mkfs desde `~/.ssh/id_ed25519.pub` o una clave de test), host key
      persistente en `/etc/ssh_host_key`, motd por SSH, `halt`, y
      `cargo xtask test` (crash-safety, boot, echo TCP, sesión SSH, apagado).

**Proyecto completo: 10/10 fases.** `cargo xtask test` verifica el sistema
de extremo a extremo; `cargo xtask run` arranca soso en QEMU.
