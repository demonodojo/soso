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
```

Salir de QEMU: `Ctrl-A X`.

## Estructura

- `kernel/` — el kernel (no_std, target x86_64-unknown-none, fuera del workspace raíz)
- `xtask/` — build de la imagen de disco (crate `bootloader`) y lanzador de QEMU
- `crates/` — (próximas fases) sosofs, soso-abi, block-dev
- `tools/` — (próximas fases) mkfs-soso
- `user/` — (próximas fases) init, shell y coreutils en userspace

## Estado

- [x] Fase 0: boot + consola serie
- [x] Fase 1: memoria e interrupciones (GDT/TSS, IDT, PIC+PIT 100 Hz, frames, heap talc)
- [x] Fase 2: kernel-shell por serie (`help uptime mem pf panic halt`)
- [x] Fase 3: PCI (ECAM) + virtio-blk (`blk blkread blkwrite`, persistente entre arranques)
- [ ] Fase 4-5: sosofs (lectura, luego escritura CoW)
- [ ] Fase 6-7: userspace (ring 3, syscalls, ELF, shell)
- [ ] Fase 8-10: red, SSH y auth
