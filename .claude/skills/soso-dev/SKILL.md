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
| `cargo xtask test` | Full integration: sosofs, boot, TCP, SSH, soso-llm, halt |
| `cargo xtask bench-llm` | Medir tok/s decode (modelo `bench`, SMP configurable) |
| `cargo xtask package-usb` | Artefactos clásicos (UEFI + data + models separados) |
| `cargo xtask package-usb-live` | Imagen live GPT única (`soso-live.img`, ver `docs/L5c-on-box.md`) |
| `cargo xtask convert-gguf` | Convert GGUF → `.som` layout (host tool) |
| `cargo xtask lx-build` | Compilar `liblxdde.a` (drivers Linux portados) |
| `cargo xtask lx-build nouveau` | Compilar solo el port nouveau/nvkm (GPU, L6/G5 GO en GB205) |
| `cargo xtask g1-check` | Checklist host G1 (IOMMU/VFIO, firmware, BAR0) |
| `cargo xtask g3-check` | Checklist bring-up GSP (firmware, módulos, fases) |
| `./scripts/l6-pack-firmware.sh` | Empaquetar firmware GSP gb205 (.zst→.bin) en rootfs |
| `./scripts/l6-g3-gsp-hostcheck.sh` | GSP hostcheck (~1 s, G4d–G4f encoders, sin GPU) |
| `./scripts/l6-g1-vfio-test.sh` | Ciclo VFIO completo (cap PCIe Gen3 antes; ver soso-gpu) |
| `./scripts/l6-h-start-cuda.sh` | L6-H nativo (requiere `llama-server` en PATH) |

**Exit QEMU:** `Ctrl-A X` (not Ctrl-C).

> **GPU / L6 (NVIDIA nouveau/GSP):** G1→G5 **GO** en GB205 (matvec en `soso-llm`) — skill **`soso-gpu`**.
> **L6-H (CUDA en host):** `docs/L6-H-cuda-hybrid.md` — `--cuda-host 10.0.2.2:11400`.

## Daily dev (GPU stays on host NVIDIA driver)

No VFIO needed for hostcheck, build, QEMU boot, or L6-H:

```sh
./scripts/l6-g3-gsp-hostcheck.sh
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask lx-build nouveau
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask run
# L6-H: see docs/L6-H-cuda-hybrid.md (Docker llama-server + cuda-proxy)
```

QEMU without passthrough shows `nvidia: sin GPU NVIDIA en PCI` — expected.

## What `run` does

1. Compiles userspace (`user/`) and copies ELFs to `rootfs/bin/`
2. Runs `mkfs-soso` on `rootfs/` → data disk image (64 MiB default)
3. Generates models disk (`target/soso-models.img`) with synthetic **tiny** model
4. Injects SSH keys into the image:
   - Authorized key: `~/.ssh/id_ed25519.pub` if present, else `target/soso_test_key`
   - Host key: persistent seed in `rootfs/etc/ssh_host_key`
5. Launches QEMU: q35, `-cpu max` (RDRAND for sunset crypto), **2 GiB RAM**,
   virtio-blk×2 + virtio-net

## Access while running

```sh
# Serial console: same terminal as cargo xtask run (sosh prompt $)

# SSH (another terminal) — -tt for tty interactiva
ssh -tt -i target/soso_test_key -p 2222 soso@localhost

# TCP echo test
nc localhost 7777
```

Port forwards (host → guest): **2222→22** (SSH), **7777→7** (echo).

Guest IP: **10.0.2.15** (DHCP; fallback estático en QEMU slirp).

## Testing

```sh
# Host-only sosofs crash-safety
cargo test -q -p sosofs --features std

# Host: soso-llm-core, sosomodel, convert-gguf
cargo test -q -p soso-llm-core -p sosomodel -p convert-gguf

# End-to-end (builds, QEMU, serial log, TCP, SSH, soso-llm via SSH, halt)
cargo xtask test

# Decode tok/s con modelo sintético bench (default SMP=1,4 mem=8G)
cargo xtask bench-llm
SOSO_BENCH_SMP=1,8 SOSO_BENCH_MAX=8 cargo xtask bench-llm

# L6-H: cuda-proxy (host; tests mockean HTTP)
cargo test -p cuda-proxy
cargo build -p cuda-proxy --release --target-dir target
```

User rule for this project: **mock HTTP and Celery calls in tests** (soso has no Celery; applies if adding HTTP client tests).

## Debugging

- Serial output is the primary console.
- `cargo xtask gdb` + remote GDB on `:1234`.
- Kernel-shell (`soso>`) is emergency fallback when userspace exits cleanly.
- QEMU uses `-no-reboot`; page faults in ring 3 kill the process, not the kernel.

## Skills layout

Skills live in `.claude/skills/`. `.cursor/skills` mirrors them — edit under `.claude/skills/`.

## Common issues

| Symptom | Fix |
|---------|-----|
| SSH permission denied | Use `-i target/soso_test_key` or ensure `~/.ssh/id_ed25519.pub` existed before build |
| Connection refused :2222 | Wait for `sosh — escribe 'help'`; or prior QEMU still running → `pkill qemu-system-x86` |
| `Could not set up host forwarding rule tcp::2222` | Puerto ocupado; `pkill qemu-system-x86` y relanzar |
| SSH output desalineada | Kernel debe enviar CRLF en `ssh::tx_push` (tty cruda) |
| SSH no reconecta tras Ctrl-C | Kernel debe hacer `reset_socket` en CloseWait/TimeWait |
| Kernel GPF al conectar SSH | Revisar alineación de pila en `timer_isr` antes de `net::poll` |
| Redirección `> file` no crea fichero | `vfs::create_file` debe delegar a sosofs (no stub) |
| RDRAND / crypto errors | QEMU must use `-cpu max` (xtask sets this) |
| Stale disk content | `cargo xtask mkfs` then re-run |
| `cuda-proxy`: binary not found | `cargo build -p cuda-proxy --release --target-dir target` |
| L6-H: connection refused :11400 | Start cuda-proxy; llama-server must answer `/health` on :8080 |
| L6-H: no tok/s from soso | Host is `10.0.2.2` from QEMU guest; model name must match loaded GGUF |
