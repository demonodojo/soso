---
name: soso-dev
description: >-
  Build, run, test and debug the soso bare-metal OS in QEMU — cargo xtask,
  mkfs, SSH access, serial console, gdb and integration tests. Use when
  starting soso, compiling the kernel or userspace, running QEMU, connecting
  by SSH, troubleshooting boot/network, or running cargo xtask test.
---

# soso — Development workflow

Minimalist Rust OS (x86_64 bare-metal) running in QEMU q35. Monousuario.

## Requirements

- **rustup** with nightly from `rust-toolchain.toml` (bootloader 0.11 needs `-Zbuild-std`)
- **qemu-system-x86_64** (`sudo apt install qemu-system-x86`)
- **OpenSSH client** + **ssh-keygen** (for SSH tests and access)

## Commands (from repo root)

| Command | Action |
|---------|--------|
| `cargo xtask build` | Compile kernel → `target/soso-bios.img` |
| `cargo xtask run` | Build + launch QEMU (serial on stdio) |
| `cargo xtask gdb` | Frozen at boot; `gdb -ex 'target remote :1234'` |
| `cargo xtask mkfs` | Force-regenerate sosofs data image from `rootfs/` |
| `cargo xtask test` | Full integration: sosofs crash tests, boot, TCP echo, SSH, halt |

**Exit QEMU:** `Ctrl-A X` (not Ctrl-C).

## What `run` does

1. Compiles userspace (`user/`) and copies ELFs to `rootfs/bin/`
2. Runs `mkfs-soso` on `rootfs/` → data disk image (64 MiB default)
3. Injects SSH keys into the image:
   - Authorized key: `~/.ssh/id_ed25519.pub` if present, else generates `target/soso_test_key`
   - Host key: persistent seed in `rootfs/etc/ssh_host_key`
4. Launches QEMU: q35, `-cpu max` (RDRAND for sunset crypto), 256M RAM, virtio-blk + virtio-net

## Access while running

```sh
# Serial console: same terminal as cargo xtask run (sosh prompt $)

# SSH (another terminal)
ssh -i target/soso_test_key -p 2222 soso@localhost

# TCP echo test
nc localhost 7777
```

Port forwards (host → guest): **2222→22** (SSH), **7777→7** (echo).

Guest IP: **10.0.2.15** (static, slirp).

## Testing

```sh
# Host-only sosofs crash-safety
cargo test -q -p sosofs --features std

# End-to-end (builds, QEMU, serial log, TCP, SSH session, halt)
cargo xtask test
```

User rule for this project: **mock HTTP and Celery calls in tests** (soso has no Celery; applies if adding HTTP client tests).

## Debugging

- Serial output is the primary console.
- `cargo xtask gdb` + remote GDB on `:1234`.
- Kernel-shell (`soso>`) is emergency fallback when userspace exits cleanly.
- QEMU uses `-no-reboot`; page faults in ring 3 kill the process, not the kernel.

## Skills layout

Skills live in `.claude/skills/`. `.cursor/skills` symlinks there — edit skills only under `.claude/skills/`.

## Common issues

| Symptom | Fix |
|---------|-----|
| SSH permission denied | Use `-i target/soso_test_key` or ensure `~/.ssh/id_ed25519.pub` existed before build |
| Connection refused :2222 | Wait for `sosh — escribe 'help'` in serial log |
| RDRAND / crypto errors | QEMU must use `-cpu max` (xtask sets this) |
| Stale disk content | `cargo xtask mkfs` then re-run |
