# soso

**soso** is a bare-metal x86_64 operating system written in Rust: own kernel,
copy-on-write filesystem with checksums (sosofs), a second read-only disk for
LLM models (sosomfs), real SSH, WiFi and Ethernet on real hardware, voice
recognition, a minimal web browser, over-the-air updates, and a native NVIDIA GPU
stack (L6). Single-user by design — one session at a time, no Unix permissions.

Use it in **QEMU** for development, on a **live USB** on real machines, or
**installed to NVMe** with UEFI dual-boot alongside Linux.

End-user guide (Spanish): [`MANUAL-USUARIO.md`](MANUAL-USUARIO.md)

## At a glance

| Area | What you get |
|------|----------------|
| **Shell** | **sosh** — pipes, redirections, `cd`/`pwd`, WiFi builtins, `ask` to the LLM |
| **LLM** | **soso-llm** + **ask** (resident daemon); models in `/models/`; MoE, MLA, Qwen, GPU offload |
| **Models** | **soso-hf** pulls GGUF from Hugging Face inside soso; host tools convert GGUF → `.som` |
| **Voice** | **soso-voz** / **voz** — Whisper ASR via mic or WAV; push-to-talk (F4) |
| **Web** | **soso-web** — HTTPS fetch, HTML→text or framebuffer GUI (no JavaScript) |
| **Install** | **soso-install** — clone live USB to NVMe from soso, no Linux required |
| **Updates** | **soso-update** — apply GitHub Releases (rootfs + kernel with rollback) |
| **Network** | DHCP, SSH-2, WiFi (Intel AX211/AX200), Realtek r8169 on live USB |
| **GPU** | lxdde + nouveau/nvkm; matvec on GB205; optional CUDA hybrid (L6-H) |

Version: [`VERSION`](VERSION) (e.g. `0.2.0`); shown at boot and in `/etc/soso-release`.

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
cargo xtask fetch-hf       # host: Hugging Face → GGUF → .som
cargo xtask convert-gguf   # host: GGUF → .som layout
cargo xtask package-usb-live   # single GPT image for live USB / install
cargo xtask release        # pack release artifacts (manifest + rootfs.pack + kernel)
```

**Live USB on real hardware:**

```sh
cargo xtask flash-usb-live /dev/sdX --yes   # flash stick (picks best model that fits)
# incremental update (kernel+rootfs only, keeps models on p3):
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models
```

With soso running (`cargo xtask run`), in another terminal:

```sh
nc localhost 7777                                              # TCP echo
ssh -tt -i target/soso_test_key -p 2222 soso@localhost         # encrypted shell
ask hola                                                       # LLM via resident askd
soso-llm run tiny --prompt hola                                # one-shot LLM inference
soso-web --local /etc/web-prueba.html                          # minimal browser
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
| `boot-shim/` | UEFI shim: BOOTMARK, install/update mailbox, chainload |
| `xtask/` | Disk images, mkfs, QEMU, releases, integration tests |
| `crates/` | sosofs, sosomfs, soso-abi, soso-llm-core, soso-http, soso-update-core, soso-web-core, soso-audio, soso-gpu, … |
| `tools/` | mkfs-soso, mkfs-sosomfs, mkmodel-soso, convert-gguf, convert-whisper, cuda-proxy |
| `user/` | libsoso, init, sosh, soso-llm, soso-voz, soso-web, soso-hf, soso-update, soso-install, coreutils |
| `rootfs/` | Source tree embedded into the data disk — see [`rootfs/README.md`](rootfs/README.md) |
| `lxdde/` | Linux-style DDE layer (`lx_emul`) for ported C drivers (e1000e, nouveau/nvkm, iwlwifi) |
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

- [x] **sosh** — pipes `|`, redirections `>`, `>>`, `<`, builtins `cd`/`pwd`/`help`/`exit`/`wifi`/`ask`/`voz`
- [x] **Per-process cwd** — relative paths resolve against the process working directory
- [x] **Coreutils** — `ls`, `cat`, `echo`, `mkdir`, `rm`, `hexdump`, `halt`
- [x] **init** — PID 1, relaunches sosh; confirms kernel updates; `init test` runs syscall regression suite
- [x] **soso-voz** — Whisper ASR daemon (`vozd`), mic capture (Intel HDA), push-to-talk
- [x] **soso-web** — HTTPS client, HTML reflow or framebuffer GUI (fontdue + DejaVu)
- [x] **soso-hf** — download/import GGUF models from Hugging Face Hub inside soso
- [x] **soso-install** — native live→NVMe installer with GPT relayout and UEFI boot entry
- [x] **soso-update** — OTA updates from GitHub Releases (rootfs pack + kernel slot with rollback)
- [x] **SIMD** — userspace AVX2+FMA; FPU state preserved with xsave64 across context switches

### LLM stack

- [x] **sosomfs** — second virtio-blk disk, model shards under `/models/<name>/`
- [x] **.som format** — manifest, index, tokenizer, quantized tensor shards (F32, Q8_0, Q4_K)
- [x] **soso-llm-core** — full Llama-style pipeline: RoPE, GQA, SwiGLU, KV cache, greedy/temp/top-p sampling, streaming decode
- [x] **soso-llm** — userspace CLI (`run`, `askd`, distributed `node`/`worker` modes)
- [x] **ask** — sosh builtin; talks to resident `askd` on `127.0.0.1:7420` (quotes and UTF-8 safe)
- [x] **Host tools** — `cargo xtask convert-gguf`, synthetic tiny/bench models via mkmodel-soso
- [x] **Custom models** — `SOSO_MODELS_DIR=<dir> cargo xtask run`
- [x] **Distributed inference** — multi-QEMU pipeline over socket netdev (`cargo xtask test-distributed-llm`)
- [x] **SMP benchmark** — `cargo xtask bench-llm` (decode tok/s vs worker count)

### Deployment, install and updates

- [x] **Classic USB package** — `cargo xtask package-usb` (UEFI + separate data/models images)
- [x] **Live USB image** — `cargo xtask package-usb-live` (single GPT stick: ESP + sosofs + sosomfs); see [`docs/L5c-on-box.md`](docs/L5c-on-box.md)
- [x] **Flash live USB** — `sudo cargo xtask flash-usb-live /dev/sdX --yes` (auto-picks largest GGUF that fits)
- [x] **Native installer** — `soso-install` from live stick: safety checks, block clone, GPT relayout, UEFI `Boot####` via boot-shim
- [x] **OTA updates** — `soso-update` from installed system; releases via `cargo xtask release [--publish]`
- [x] **Versioning** — `VERSION` file → `/etc/soso-release` + kernel banner; semver compare in updates
- [x] **Dual-boot UEFI + Linux** — `sudo cargo xtask install-disk /dev/nvmeXn1 --yes` (host-side), or native install
- [x] **QEMU live mode** — `SOSO_QEMU_LIVE=1 cargo xtask run`; install flow: `cargo xtask test-install`; update flow: `cargo xtask test-update`
- [x] **Persistent logs** — `cargo xtask sosolog` reads `SOSOLOG.TXT` / `SOSODRV.TXT` from live ESP

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
| `cargo xtask package-usb-live` | Single GPT image (ESP + sosofs + sosomfs) for USB or install |
| `sudo cargo xtask flash-usb-live /dev/sdX --yes` | Flash live USB (model sized to stick) |
| `sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models` | Incremental: ESP + rootfs only (models unchanged) |
| `cargo xtask release [--publish]` | Pack release (`manifest.txt`, `rootfs.pack`, `kernel-x86_64`); `--publish` → GitHub Releases |
| `cargo xtask test-install` | E2E native install (OVMF, 3 boots) |
| `cargo xtask test-update` | E2E local update (`soso-update aplicar --local`) |
| `cargo xtask sosolog [--drv]` | Read `SOSOLOG.TXT` / `SOSODRV.TXT` from live USB ESP |
| `sudo cargo xtask install-disk /dev/nvmeXn1 --yes` | Dual-boot: write live image to empty disk + GRUB entry |
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
