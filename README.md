# soso

**soso** is a bare-metal x86_64 operating system written in Rust: own kernel,
copy-on-write filesystem with checksums (sosofs), a second read-only disk for
LLM models (sosomfs), SSH, Ethernet and WiFi drivers, voice recognition, a
minimal web browser, native install, over-the-air updates, and an NVIDIA GPU
stack (L6). Single-user by design — no Unix permissions, up to **four** SSH
sessions at once.

Use it in **QEMU** for development, on a **live USB** on real machines, or
**installed to NVMe** with UEFI dual-boot alongside Linux.

Tree version: **0.3.7** ([`VERSION`](VERSION)), shown at boot and in
`/etc/soso-release`.

End-user guide (Spanish): [`MANUAL-USUARIO.md`](MANUAL-USUARIO.md).

Operational docs (Spanish): [`docs/GUIA-OPERATIVA.md`](docs/GUIA-OPERATIVA.md),
[`docs/ESTADO.md`](docs/ESTADO.md). Hardware matrix:
[`docs/HW-MATRIX.md`](docs/HW-MATRIX.md).

## At a glance

| Area | What you get |
|------|----------------|
| **Shell** | **sosh** — pipes, and-or lists, redirections, Spanish keymap, `ask` / `voz` / `wifi` |
| **LLM** | **soso-llm** + resident **askd**; models in `/models/`; MoE, MLA, Qwen; GPU offload when a VRAM pool exists |
| **Models** | **soso-hf** pulls GGUF from Hugging Face; **ask-modelo** picks the default; host tools convert GGUF → `.som` |
| **Voice** | **soso-voz** / **voz** — Whisper ASR via Intel HDA or a WAV; push-to-talk (F4) |
| **Web** | **soso-web** — HTTPS fetch, HTML→text or framebuffer GUI (no JavaScript) |
| **Install** | **soso-install** — clone the live USB to NVMe from soso |
| **Disk** | **soso-resize** — grow sosofs by taking free space from the end of the models partition |
| **Updates** | **soso-update** — GitHub Releases: kernel with verifiable rollback; rootfs file-by-file, retryable, no automatic rollback of old binaries |
| **Network** | DHCP, SSH-2 (4 sessions), virtio-net / e1000e / Realtek r8169, Intel AX211/AX200 driver |
| **GPU** | lxdde + nouveau/nvkm; G1–G5 historically GO on GB205 (revalidation still open); optional CUDA hybrid (L6-H) |
| **Self-hosting** | **soso-ed**, remote **soso-forja**, **soso-git** content hashes. Native `rustc` is still a stub |

## Where things stand (September 2026)

QEMU is the reliable path: boot, sosofs, SSH, syscall regression, LLM smoke
(`tiny`), native install, OTA (apply, kernel recovery, invalid manifest), USB
mass storage, and the remote forge loop are covered by `cargo xtask`. Cycle B
(resize, ELF/argv, remote forge, CI, the hardware-matrix parser, GPT cache) is
closed for those host and QEMU checks. Re-validating a board (B5) is still
open. Detail and the published OTA limits live in
[`docs/ESTADO.md`](docs/ESTADO.md).

On real hardware, treat [`docs/hw-matrix.json`](docs/hw-matrix.json) as the
record. A historical GO is not a fresh `ok`:

| Piece | In the tree | Still open on hardware |
|-------|-------------|------------------------|
| Live USB, GOP console, NVMe, xHCI | Used on ROG / GB205 boards | Each board needs its own matrix entry |
| NVIDIA GB205 | G1–G5 GO logged in July 2026 | Revalidate after FWSEC/falcon changes |
| NVIDIA Ampere (GA104 `10de:249c` on the ROG) | FWSEC-FRTS path written | GPU stages are not `ok` in the matrix |
| Ethernet Realtek r8169 | Native driver, DHCP, no slirp fallback | — |
| WiFi AX200 | ALIVE, init, MVM and scan on the ROG | SCD, data path and WPA2 against an AP |
| WiFi AX211 | Driver + gen3 descriptor fix | Real ALIVE (last board log timed out before that fix) |
| Steam Deck OLED | Defensive AMD-Vi shutdown, rotated 800×1280 console, USB input plumbing, ath11k MHI hostcheck | Never booted on the Deck. QMI framing exists; HTC/WMI, data rings, scan and WPA2 do not. RDNA2 is out of scope |
| Self-improvement (OpenCode + local model) | Coordinador, chat API and many Txx cards in Rust | No SI-0–SI-7 milestone closed; no accepted first patch. See [`SELF_IMPROVEMENT.md`](SELF_IMPROVEMENT.md) |
| Native toolchain | PAL, `soso-std`, `soso-rt`, `soso-alloc`, bootstrap scripts | `soso-rustc` is a stub; libstd still fails after the allocator |

Logs: live ESP keeps `SOSOLOG.TXT` / `SOSODRV.TXT` (`cargo xtask sosolog`).
An installed system also writes `/var/log/{kernel,aplicaciones,actualizaciones}.log`.

## Requirements

| Tool | Purpose |
|------|---------|
| **rustup** | Builds kernel and userspace (nightly pinned in `rust-toolchain.toml`; required because bootloader 0.11 uses `-Zbuild-std`) |
| **qemu-system-x86_64** | Run the VM (`sudo apt install qemu-system-x86`) |
| **OVMF** | `test-install`, `test-update` and `test-resize` boot a UEFI guest |
| **OpenSSH client** + **ssh-keygen** | SSH access and integration tests |
| **clang**, **zstd** | Optional — lxdde ports and NVIDIA / ath11k firmware packing |

## Quick start

```sh
cargo xtask build          # compile kernel → target/soso-bios.img
cargo xtask check          # host tests + builds + iwl/GSP/ath11k hostchecks + hw-matrix parser
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
cargo xtask flash-usb-live /dev/sdX --yes --skip-models
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

Guest network: DHCP at boot. Fallback **10.0.2.15/24** only for virtio-net and
e1000e under QEMU slirp — not for WiFi and not for rtl8169. Port forwards:
**2222→22** (SSH), **7777→7** (echo). `ask` and `voz` talk to local daemons on
`127.0.0.1` (kernel loopback, no extra NIC).

## Project layout

| Path | Role |
|------|------|
| `kernel/` | `no_std` kernel (`x86_64-unknown-none`, outside the root workspace) |
| `boot-shim/` | UEFI shim: BOOTMARK, install/update mailbox, chainload |
| `xtask/` | Disk images, mkfs, QEMU, releases, integration tests |
| `crates/` | sosofs, sosomfs, soso-abi, soso-llm-core, soso-llm-api, soso-http, soso-update-core, soso-web-core, soso-audio, soso-gpu, soso-std, soso-rt, soso-alloc, soso-improve-core, gptdisk, xhci-nostd, … |
| `tools/` | mkfs-soso, mkfs-sosomfs, mkmodel-soso, convert-gguf, convert-whisper, cuda-proxy, soso-forja-server, sosoas, wild-soso |
| `user/` | libsoso, init, sosh, soso-llm, soso-voz, soso-web, soso-hf, soso-update, soso-install, soso-ed, soso-forja, soso-git, soso-improve, coreutils |
| `config/rust-soso/` | Rust std PAL and bootstrap patches for a future native toolchain |
| `rootfs/` | Source tree embedded into the data disk — see [`rootfs/README.md`](rootfs/README.md) |
| `lxdde/` | Freestanding C ports: e1000e, nouveau/nvkm, iwlwifi, ath11k |
| `docs/` | Status, hardware matrix, live USB, OTA, self-hosting, L6, Steam Deck |

## What is implemented

### Kernel and platform

- [x] **Boot** — BIOS/UEFI via bootloader 0.11, serial and GOP framebuffer, q35 + `-cpu max`
- [x] **Memory** — GDT/TSS, IDT, PIC+PIT 100 Hz, frame allocator, buddy heap, clock reclaim for read-only mmap
- [x] **PCI** — ECAM enumeration; `hwscan` prints one line per device on every boot
- [x] **Block I/O** — virtio-blk (data + models), NVMe (512- and 4096-byte LBA), live GPT reader, USB mass storage + xHCI
- [x] **Filesystem** — sosofs CoW B+ tree, CRC32C, dual superblocks, commits on `close()` and about every 2 s; `NAME_MAX` 255
- [x] **VFS** — `/models/*` → sosomfs (read-only catalog; atomic import via `SYS_SOM_*`); everything else → sosofs
- [x] **Userspace** — ring 3 ELF, preemptive round-robin, SMP (`-smp N`), **spawn** (no fork), user threads joined by futex
- [x] **Syscalls** — file I/O, mmap/munmap, pipe, spawn/spawn_io, chdir/getcwd, futex, signals (delivery only, no handlers), GPU, TCP, audio, framebuffer
- [x] **Console** — GOP framebuffer, UTF-8, Spanish ISO-105 keymap by default (`kbd es` / `kbd us` in the kernel shell)
- [x] **Power** — `halt` (ACPI S5 on hardware; isa-debug-exit in QEMU) and `reboot`
- [x] **Network** — smoltcp TCP/IPv4, DHCP, polled NICs: virtio-net, native e1000e, rtl8169, iwlwifi
- [x] **SSH** — sunset (curve25519, ed25519, chacha20-poly1305), four sessions, reconnectable, CRLF on the remote tty
- [x] **Emergency kshell** — `soso>` (`help`, `dmesg`, `hwscan`, `ip`, `ping`, `wifi`, `io`, `halt`, `reboot`, …) if userspace exits

### Shell and userspace

- [x] **sosh** — pipes `|`, redirections including `N>&M`, lists with `;` / `&&` / `||`, builtins `cd` / `pwd` / `help` / `exit` / `wifi` / `ask` / `voz`
- [x] **Per-process cwd** — relative paths resolve against the process working directory
- [x] **Coreutils** — `ls`, `cat`, `echo`, `mkdir`, `rm`, `cp`, `mv`, `hexdump`, `grep`, `diff`, `find`, `wc`, `head`, `tail`, `stat`, `ip`, `ping`, `dns`, `ps`, `log`, `halt`, `reboot`
- [x] **init** — PID 1, relaunches sosh, confirms a kernel update only after rootfs and `/tmp/sosh-ready`; `init test` is the syscall regression suite
- [x] **soso-voz** — Whisper ASR daemon (`vozd` on `127.0.0.1:7421`); transcription is inserted into the line and never run by itself
- [x] **soso-web** — HTTPS client, HTML reflow or framebuffer GUI (fontdue + DejaVu)
- [x] **soso-hf** — download and atomically import GGUF models from Hugging Face inside soso
- [x] **soso-install** / **soso-resize** — live→NVMe installer (GPT relayout, UEFI boot entry via the shim); grow the root filesystem on a GPT disk
- [x] **soso-update** — kernel slot with `SOSOKRN.MET` recovery; rootfs pack applied per file with progress in `/etc/actualiza.estado`
- [x] **SIMD** — userspace AVX2+FMA; the kernel saves x87/XMM/YMM with `xsave64`

### LLM stack

- [x] **sosomfs** — model shards under `/models/<name>/`
- [x] **.som format** — manifest (through v4: GQA, MoE, MLA, Qwen gated / GDN), index, tokenizer, shards F32 / Q8_0 / Q4_K / MXFP4
- [x] **soso-llm-core** — RoPE, GQA, SwiGLU, KV cache, sampling, streaming decode; same runtime on host and in the guest
- [x] **soso-llm** — `run`, resident `askd`, distributed `node` / `worker`
- [x] **ask** — sosh builtin; one question at a time to `askd` on `127.0.0.1:7420` (quotes and UTF-8 stay intact)
- [x] **Host tools** — `cargo xtask convert-gguf`, `fetch-hf`, `fetch-whisper`; synthetic models via mkmodel-soso
- [x] **Custom models** — `SOSO_MODELS_DIR=<dir> cargo xtask run`
- [x] **Distributed inference** — multi-QEMU pipeline (`cargo xtask test-distributed-llm`)
- [x] **Chat HTTP API** — `soso-llm-api` (guest evidence exists; model-quality gate T14 is not GO)

### Self-hosting

Remote forge loop is closed (milestone B3): the guest uploads sources, the host
builds in an isolated tree, and the client checks the receipt before writing
anything. See [`docs/SELF-HOSTING.md`](docs/SELF-HOSTING.md).

- [x] **soso-ed** — edit sources under `/src/soso`
- [x] **soso-forja** — `local` only plans; `sync|build|all --host` builds on the host; `build-local` copies artifacts already in `/var/forja-out`
- [x] **soso-git** — `status|log|diff|commit` as content hashes (not a full git client)
- [x] **soso-std** / **hola-std** — small standard library and a guest smoke demo
- [ ] **Native rustc** — `soso-rustc` prints a version and checks for a sysroot. Bootstrap (`cargo xtask rust-bootstrap`) is in progress; libstd still fails after the allocator

## lxdde and native GPU (L6)

Optional kernel feature (`SOSO_LXDDE=1`) linking a freestanding C library built
from ported Linux 6.6 driver code:

| Port | Status |
|------|--------|
| `spike`, `testdrv` | DDE plumbing validated |
| `e1000e` | Linux-style NIC (`SOSO_QEMU_NIC=lx-e1000e`). The live image also has a native e1000e |
| `nouveau` | G1–G5 **historically GO** on GB205 (GSP-FMC → RM → CE → SASS matvec). Revalidation after later firmware work is still open. Ampere path written, not `ok` in the matrix |
| `iwlwifi` | Intel AX211 (gen3) and AX200 (gen2). See the hardware table above |
| `ath11k` | Steam Deck WCN6855: MHI hostcheck green against a model; not proven on silicon |

GPU syscalls: `SYS_GPU_INFO`, `SYS_GPU_ALLOC`, `SYS_GPU_MAP`, `SYS_GPU_SUBMIT`,
`SYS_GPU_READ`. Pack firmware with `./scripts/l6-pack-firmware.sh` (NVIDIA) or
`./scripts/l6-pack-ath11k-fw.sh` (Deck).

**L6 roadmap (G1→G5):**

| Gate | Deliverable | Status |
|------|-------------|--------|
| G1 | VFIO passthrough + BAR0 (`NV_PMC_BOOT_0`) | **Done** on real GB205 (`0x1b5000a1`); revalidation open |
| G2 | GSP firmware blobs in sosofs | **Done** (gb205 + ga102 reference set) |
| G3a | ELF validation, GEM staging | **Done** |
| G3b | Real GSP boot (FSP/COT, radix3, WPR, libos) | **Done** on GB205 HW; revalidation open |
| G4a–c | GSP-RM RPC + RM objects | **Done** on GB205 HW; revalidation open |
| G4d | VRAM + external VA space (VER3 page tables) | **Done** on GB205 HW (exercised by CE/compute) |
| G4e | GPFIFO channel + CE copy | **Done** (2026-07-29): `CE readback verificado (G4e GO)` |
| G4f | SAXPY / matvec SASS (`SYS_GPU_SUBMIT`) | **Done** (2026-07-29): PCAS 24 B + QMD v05, `on_gpu=1` |
| G5 | Hybrid LLM matvec on GPU | **GO functional** (2026-07-29) on `tiny`; large-model tok/s still TBD; revalidation open |
| **L6-H** | CUDA inference on host Linux (`--cuda-host`) | **GO** (2026-07-27): cuda-proxy + llama-server. This is host CUDA, not the in-kernel GPU path |

Details: [`docs/L6-native-autonomy.md`](docs/L6-native-autonomy.md),
[`docs/L6-G1-gate.md`](docs/L6-G1-gate.md),
[`docs/L6-G3-nvkm-scope.md`](docs/L6-G3-nvkm-scope.md),
[`docs/L6-H-cuda-hybrid.md`](docs/L6-H-cuda-hybrid.md).

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
| `cargo xtask mkfs` | Force-regenerate the sosofs image from `rootfs/` |
| `cargo xtask package-usb-live` | Single GPT image (ESP + sosofs + sosomfs) |
| `cargo xtask flash-usb-live /dev/sdX --yes` | Flash a live USB (model sized to the stick) |
| `cargo xtask flash-usb-live /dev/sdX --yes --skip-models` | Incremental: ESP + rootfs only |
| `cargo xtask release [--publish]` | Pack a release; `--publish` uploads a GitHub Release |
| `cargo xtask test-install` | E2E native install (OVMF, 3 boots) |
| `cargo xtask test-update` | E2E OTA: apply, kernel recovery, invalid manifest |
| `cargo xtask test-resize` | Grow/recovery of sosofs (host cuts + QEMU) |
| `cargo xtask test-usb` | USB mass storage / xHCI |
| `cargo xtask fb-shot` | Capture the guest framebuffer |
| `cargo xtask check` | Pre-commit: host tests, builds, iwl/GSP/ath11k hostchecks, hw-matrix parser |
| `cargo xtask hw-matrix show` | Hardware validation matrix |
| `cargo xtask sosolog [--drv]` | Read `SOSOLOG.TXT` / `SOSODRV.TXT` from the live ESP |
| `cargo xtask install-disk /dev/nvmeXn1 --yes` | Dual-boot: write the live image and a GRUB entry |
| `cargo xtask lx-build [port\|all]` | Build `liblxdde.a` (spike, testdrv, e1000e, nouveau, iwlwifi, ath11k) |
| `cargo xtask bench-llm` | Decode tok/s under configurable SMP |
| `cargo xtask test-distributed-llm` | Two-QEMU distributed LLM smoke test |
| `cargo xtask test-llm-api` | Chat HTTP API checks |
| `cargo xtask sync-src` | Copy editable sources to `/src/soso` in the image |
| `cargo xtask forja-out` | Copy release artifacts into `rootfs/var/forja-out/` |
| `cargo xtask rust-bootstrap` | Apply the std PAL patches and start a Rust bootstrap |
| `./scripts/l6-pack-firmware.sh` | Pack NVIDIA GSP firmware into rootfs |
| `./scripts/l6-g3-gsp-hostcheck.sh` | GSP bring-up hostcheck (no GPU) |
| `./scripts/l6-iwl-fw-hostcheck.sh` | iwlwifi firmware parser hostcheck |
| `./scripts/l6-h-start-cuda.sh` | L6-H: llama-server + cuda-proxy |

Useful environment variables:

```sh
SOSO_MODELS_DIR=/path/to/model          # custom .som tree on the models disk
SOSO_QEMU_GPU=vfio:01:00.0              # GPU passthrough (requires IOMMU)
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau    # enable a port; live USB uses nouveau,iwlwifi
SOSO_QEMU_LIVE=1                        # boot the live GPT image in QEMU
SOSO_QEMU_NIC=lx-e1000e                 # lxdde e1000e instead of virtio-net
SOSO_DRIVERS=qemu|live-usb|all          # which drv-* features to compile
SOSO_LIVE_OFFLINE=1                     # package a stick without fetching a model
SOSO_ROOTFS_SIZE=64G                    # data image size (default 32G)
```

## Testing

| Layer | Command |
|-------|---------|
| All host checks + builds | `cargo xtask check` |
| sosofs crash-safety (host) | `cargo test -p sosofs --features std` |
| sosomfs (host) | `cargo test -p sosomfs --features std` |
| OTA recovery (host) | `cargo test -p soso-update-core --features std --tests` |
| Syscall regression (guest) | `/bin/init test` in QEMU |
| Full system E2E | `cargo xtask test` |
| OTA E2E (OVMF) | `cargo xtask test-update` |
| USB/xHCI | `cargo xtask test-usb` |
| Native install E2E | `cargo xtask test-install` |
| Rootfs resize | `cargo xtask test-resize` |

`cargo xtask test` covers host filesystem tests, QEMU boot to sosh, TCP echo,
an authenticated SSH session, `soso-llm run tiny`, LLM shutdown, and `halt`.
A red suite is not an acceptable release. Investigate the failure.

Do not run `sudo cargo` — root has no rustup. `flash-usb-live` and
`install-disk` ask for sudo only when writing the disk.

## License

Copyright (C) 2026 Jose Miguel Díez de la Lastra Jimeno.

First-party soso code (kernel, crates, userspace, tools, xtask) is distributed
under **GNU General Public License v2.0 only** (GPL-2.0-only). See [`COPYING`](COPYING).

Third-party components with their own licenses: [`THIRD_PARTY.md`](THIRD_PARTY.md)
(including optional NVIDIA firmware under proprietary terms).
