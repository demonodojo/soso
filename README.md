# soso

A minimalist learning operating system in Rust: bare-metal x86_64 kernel, custom
copy-on-write filesystem with checksums (sosofs), LLM model storage on a second
disk (sosomfs), real SSH access, and a growing native NVIDIA GPU stack (L6).
Single-user by design.

**Goal:** boot in QEMU, connect with `ssh -p 2222 localhost` using an ed25519 key,
use **sosh** with pipes, redirections, and per-process working directories, and run
LLM inference with **soso-llm**.

End-user guide: [`MANUAL-USUARIO.md`](MANUAL-USUARIO.md) (Spanish).

## Requirements

| Tool | Purpose |
|------|---------|
| **rustup** | Builds kernel and userspace (nightly pinned in `rust-toolchain.toml`; required because bootloader 0.11 uses `-Zbuild-std`) |
| **qemu-system-x86_64** | Run the VM (`sudo apt install qemu-system-x86`) |
| **OpenSSH client** + **ssh-keygen** | SSH access and integration tests |
| **clang**, **zstd** | Optional — lxdde GPU/NIC ports and NVIDIA firmware packing |

## Quick start

```sh
cargo xtask build          # compile kernel → target/soso-bios.img
cargo xtask run            # build + QEMU q35, serial console on stdio
cargo xtask gdb            # like run, frozen at boot; gdb -ex 'target remote :1234'
cargo xtask test           # integration: FS, boot, TCP, SSH, soso-llm, halt
cargo xtask convert-gguf   # host: GGUF → .som layout
```

With soso running (`cargo xtask run`), in another terminal:

```sh
nc localhost 7777                                              # TCP echo
ssh -tt -i target/soso_test_key -p 2222 soso@localhost         # encrypted shell
soso-llm run tiny --prompt hola                                # LLM inference
soso-llm run tinyllama-q4km --cuda-host 10.0.2.2:11400 --prompt hola --max 32   # L6-H (host CUDA)
```

**Exit QEMU:** `Ctrl-A X` (not `Ctrl-C`). If port 2222 is busy:
`pkill qemu-system-x86` before restarting.

Guest network: DHCP at boot, fallback **10.0.2.15/24** in QEMU slirp. Port forwards:
**2222→22** (SSH), **7777→7** (echo).

## Project layout

| Path | Role |
|------|------|
| `kernel/` | `no_std` kernel (`x86_64-unknown-none`, outside root workspace) |
| `xtask/` | Disk images, mkfs, QEMU launcher, integration tests |
| `crates/` | sosofs, sosomfs, soso-abi, soso-llm-core, sosomodel, block-dev, soso-gpu, xhci-nostd |
| `tools/` | mkfs-soso, mkfs-sosomfs, mkmodel-soso, convert-gguf, ssh-proto, cuda-proxy |
| `user/` | libsoso, init, sosh, coreutils, soso-llm |
| `rootfs/` | Source tree embedded into the data disk by mkfs-soso — see [`rootfs/README.md`](rootfs/README.md) |
| `lxdde/` | Linux-style DDE layer (`lx_emul`) for ported C drivers (e1000e, nouveau/nvkm) |
| `docs/` | Architecture notes (L5c on-box, L6 GPU roadmap) |

## What is implemented

### Kernel and platform (phases 0–10)

- [x] **Boot** — BIOS/UEFI via bootloader 0.11, serial console, q35 + `-cpu max`
- [x] **Memory** — GDT/TSS, IDT, PIC+PIT 100 Hz, frame allocator, buddy heap
- [x] **PCI** — ECAM enumeration
- [x] **Block I/O** — virtio-blk (data + models disks), NVMe, live GPT disk reader, USB mass storage + xHCI
- [x] **Filesystem** — sosofs CoW B+ tree, CRC32C checksums, dual superblocks, commits on `close()` and periodic flush
- [x] **VFS** — routes `/models/*` → sosomfs (read-only); everything else → sosofs
- [x] **Userspace** — ring 3 ELF processes, preemptive round-robin scheduler, **spawn** (no fork)
- [x] **Syscalls** — file I/O, mmap/munmap, pipe, spawn/spawn_io, chdir/getcwd, sleep, halt, futex, GPU ioctls
- [x] **Network** — smoltcp TCP/IPv4, DHCP client, polled virtio-net (and optional lxdde e1000e NIC)
- [x] **SSH** — sunset stack (curve25519, ed25519, chacha20-poly1305), single session, reconnectable, CRLF on remote tty
- [x] **Emergency kshell** — serial diagnostic shell (`soso>`) if userspace exits

### Shell and userspace

- [x] **sosh** — pipes `|`, redirections `>`, `>>`, `<`, builtins `cd`/`pwd`/`help`/`exit`
- [x] **Per-process cwd** — relative paths resolve against the process working directory
- [x] **Coreutils** — `ls`, `cat`, `echo`, `mkdir`, `rm`, `hexdump`, `halt`
- [x] **init** — PID 1, relaunches sosh; `init test` runs syscall regression suite
- [x] **SIMD** — userspace built with AVX2+FMA kernels in soso-llm-core; FPU state preserved with xsave64 across context switches

### LLM stack

- [x] **sosomfs** — second virtio-blk disk, model shards under `/models/<name>/`
- [x] **.som format** — manifest, index, tokenizer, quantized tensor shards (F32, Q8_0, Q4_K)
- [x] **soso-llm-core** — full Llama-style pipeline: RoPE, GQA, SwiGLU, KV cache, greedy/temp/top-p sampling, streaming decode
- [x] **soso-llm** — userspace CLI (`run`, distributed `node`/`worker` modes)
- [x] **Host tools** — `cargo xtask convert-gguf`, synthetic tiny/bench models via mkmodel-soso
- [x] **Custom models** — `SOSO_MODELS_DIR=<dir> cargo xtask run`
- [x] **Distributed inference** — multi-QEMU pipeline over socket netdev (`cargo xtask test-distributed-llm`)
- [x] **SMP benchmark** — `cargo xtask bench-llm` (decode tok/s vs worker count)

### Deployment and packaging

- [x] **Classic USB package** — `cargo xtask package-usb` (UEFI + separate data/models images)
- [x] **Live USB image** — `cargo xtask package-usb-live` (single GPT stick: ESP + sosofs + sosomfs); see [`docs/L5c-on-box.md`](docs/L5c-on-box.md)
- [x] **QEMU live mode** — `SOSO_QEMU_LIVE=1 cargo xtask run`

### lxdde and native GPU (L6 — G1→G5 GO on GB205)

Optional kernel feature (`SOSO_LXDDE=1`) linking a freestanding C library built from
ported Linux driver code:

| Port | Status |
|------|--------|
| `spike`, `testdrv` | DDE plumbing validated |
| `e1000e` | Linux-style NIC backend (`SOSO_QEMU_NIC=lx-e1000e`) |
| `nouveau` | **G5 GO** on GB205 (GSP-FMC → RM → CE → SASS matvec in `soso-llm`); Ampere GA10x path written, untested |

GPU syscalls: `SYS_GPU_INFO`, `SYS_GPU_ALLOC`, `SYS_GPU_MAP`, `SYS_GPU_SUBMIT`,
`SYS_GPU_READ`. Firmware is packed into rootfs with `./scripts/l6-pack-firmware.sh`.

**L6 roadmap (G1→G5):**

| Gate | Deliverable | Status |
|------|-------------|--------|
| G1 | VFIO passthrough + BAR0 (`NV_PMC_BOOT_0`) | **Done** on real GB205 (`0x1b5000a1`) |
| G2 | GSP firmware blobs in sosofs | **Done** (gb205 + ga102 reference set) |
| G3a | ELF validation, GEM staging | **Done** |
| G3b | Real GSP boot (FSP/COT, radix3, WPR, libos) | **Done** on GB205 HW |
| G4a–c | GSP-RM RPC + RM objects | **Done** on GB205 HW |
| G4d | VRAM + external VA space (VER3 page tables) | **Done** on GB205 HW (exercised by CE/compute) |
| G4e | GPFIFO channel + CE copy (`gsp_chan`, `gsp_ce`) | **Done** (2026-07-29): `CE readback verificado (G4e GO)` |
| G4f | SAXPY / matvec SASS (`SYS_GPU_SUBMIT`) | **Done** (2026-07-29): PCAS 24 B + QMD v05, `on_gpu=1` |
| G5 | Hybrid LLM matvec on GPU | **GO functional** (2026-07-29): `soso-llm run tiny` → matvec on GPU; tok/s on large models still TBD |
| **L6-H** | CUDA inference on host Linux (`--cuda-host`) | **GO** (2026-07-27): ~35 tok/s via cuda-proxy + llama-server |

Details: [`docs/L6-native-autonomy.md`](docs/L6-native-autonomy.md), [`docs/L6-G1-gate.md`](docs/L6-G1-gate.md), [`docs/L6-G3-nvkm-scope.md`](docs/L6-G3-nvkm-scope.md), [`docs/L6-H-cuda-hybrid.md`](docs/L6-H-cuda-hybrid.md). Skill: `.cursor/skills/soso-gpu/` (or `.claude/skills/soso-gpu/`).

**Daily dev without releasing the GPU** (driver stays on the host):

| Step | Command |
|------|---------|
| Hostcheck (~1 s) | `./scripts/l6-g3-gsp-hostcheck.sh` |
| Build nouveau | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask lx-build nouveau` |
| Build soso | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build` |
| QEMU (no passthrough) | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask run` |
| L6-H host stack | See [`docs/L6-H-cuda-hybrid.md`](docs/L6-H-cuda-hybrid.md) |

## Build commands

| Command | Action |
|---------|--------|
| `cargo xtask mkfs` | Force-regenerate sosofs data image from `rootfs/` |
| `cargo xtask lx-build [port\|all]` | Build `liblxdde.a` (spike, testdrv, e1000e, nouveau) |
| `cargo xtask g1-check` | Host checklist: IOMMU/VFIO, firmware, BAR0 |
| `cargo xtask g3-check` | GSP bring-up checklist (firmware, modules, phases) |
| `cargo xtask bench-llm` | Measure decode tok/s under configurable SMP |
| `cargo xtask test-distributed-llm` | Two-QEMU distributed LLM smoke test |
| `./scripts/l6-pack-firmware.sh` | Pack NVIDIA GSP firmware (.zst→.bin) into rootfs |
| `./scripts/l6-g3-gsp-hostcheck.sh` | GSP bring-up hostcheck (steps 3–6 + G4d–G4f encoders, no GPU) |
| `./scripts/l6-h-start-cuda.sh` | L6-H: llama-server (native) + cuda-proxy (requires `llama-server` in PATH) |
| `./scripts/l6-g1-vfio-persist.sh` | Persistent VFIO bind for iterative G1–G5 / VFIO cycles |
| `./scripts/l6-g1-vfio-test.sh` | Full VFIO cycle (PCIe Gen3 cap + Gen4/5 bump GO; see soso-gpu skill) |

Useful environment variables:

```sh
SOSO_MODELS_DIR=/path/to/model     # custom .som tree on models disk
SOSO_QEMU_GPU=vfio:01:00.0       # GPU passthrough (requires IOMMU)
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau   # enable nouveau/nvkm port
SOSO_QEMU_LIVE=1                   # boot from live GPT image in QEMU
SOSO_QEMU_NIC=lx-e1000e            # use lxdde e1000e instead of virtio-net
```

## Testing

| Layer | Command |
|-------|---------|
| sosofs crash-safety (host) | `cargo test -p sosofs --features std` |
| sosomfs (host) | `cargo test -p sosomfs --features std` |
| Syscall regression (guest) | `/bin/init test` in QEMU |
| Full system E2E | `cargo xtask test` |

`cargo xtask test` verifies: host FS tests, QEMU boot to sosh, TCP echo, SSH
authenticated session, `soso-llm run tiny`, and clean shutdown via `halt`.

## License

Copyright (C) 2026 Jose Miguel Díez de la Lastra Jimeno.

First-party soso code (kernel, crates, userspace, tools, xtask) is distributed
under **GNU General Public License v2.0 only** (GPL-2.0-only). See [`COPYING`](COPYING).

Third-party components with their own licenses: [`THIRD_PARTY.md`](THIRD_PARTY.md)
(including optional NVIDIA firmware under proprietary terms).
