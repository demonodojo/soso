---
name: soso-architecture
description: >-
  soso OS architecture — kernel layout, sosofs CoW filesystem, syscalls,
  userspace ABI, networking, SSH stack and coding constraints. Use when
  modifying kernel/, crates/, user/, xtask/, adding features, syscalls,
  drivers, or understanding how components interact.
---

# soso — Architecture

Learning OS in Rust. **10/10 phases complete.** Bare-metal x86_64 on QEMU q35.

## Workspace layout

```
soso/
├── kernel/           # no_std kernel (x86_64-unknown-none, outside root workspace)
├── xtask/            # build image, mkfs, QEMU launcher, integration tests
├── crates/
│   ├── sosofs/       # CoW FS (no_std + "std" feature for host tests)
│   ├── sosomfs/      # read-only model shards FS (second virtio-blk)
│   ├── soso-abi/     # syscall numbers, Stat, Dirent, errno
│   ├── soso-llm-core/  # inference runtime (host + userspace)
│   ├── sosomodel/    # .som manifest/index/shard format
│   └── block-dev/    # BlockDevice trait (virtio-blk, host File)
├── tools/
│   ├── mkfs-soso/    # host: rootfs dir → sosofs image + SSH key injection
│   ├── mkfs-sosomfs/ # host: model tree → models disk image
│   ├── convert-gguf/ # host: GGUF → .som layout
│   └── ssh-proto/    # host prototype (phase 9), isolated from workspace
├── user/             # userspace workspace (libsoso, init, sosh, coreutils)
└── rootfs/           # source tree embedded into disk by mkfs-soso
```

## Kernel (monolithic, single-core)

- Ring 0: drivers, FS, network, SSH server — kernel never preempted
- Ring 3: ELF processes, round-robin preemptive scheduler
- **spawn, not fork** — one PML4 per process, static ELFs at `0x400000`
- Entry: `syscall`/`sysret` (MSRs STAR/LSTAR)

### Syscalls principales

`exit, read, write, open, close, seek, stat, getdents, mkdir, unlink, spawn, wait, sbrk, sleep_ms, halt, mmap, munmap, pipe, spawn_io, chdir, getcwd` (+ GPU)

- **Pipes/redirecciones:** sosh usa `pipe` + `spawn_io`; hijos heredan cwd del padre
- **Escritura:** `open(O_WRONLY)` → buffer en kernel; `create_file` en sosofs al `close()`
- **Rutas:** `task/path.rs` resuelve relativas contra `Process.cwd` (default `/`)
- Sin signals ni permisos Unix

### Key subsystems

| Module | Role |
|--------|------|
| `arch/` | GDT/TSS, IDT, PIC+PIT 100 Hz, paging |
| `drivers/` | serial, virtio-blk, virtio-net (PCI ECAM) |
| `fs/` | sosofs (blk0) + sosomfs (blk1); VFS enruta `/models/*` |
| `vfs.rs` | Router: lectura/escritura sosofs; modelos → sosomfs (read-only) |
| `net/` | smoltcp, DHCPv4 al arrancar (fallback 10.0.2.15), polled from scheduler |
| `net/ssh.rs` | sunset SSH-2, una sesión, CRLF en tx_push, reset_socket al desconectar |
| `kshell.rs` | Emergency kernel-shell on serial (`soso>`) |
| `task/` | Processes (cwd, console), scheduler, syscall, path normalization |

## sosofs (v1)

- 4 KiB blocks, little-endian, full CoW B+ tree (no journal)
- Dual superblocks A/B — atomic commit via generation increment
- CRC32C checksums on all nodes and file extents
- **Host-first development**: `cargo test -p sosofs --features std` with crash-injection before kernel integration
- Commits on write `close()` and every ~2 s

## Userspace

| Binary | Role |
|--------|------|
| `/bin/init` | PID 1: spawns sosh, relaunches on crash; `init test` = syscall regression suite |
| `/bin/sosh` | Shell: pipes, redirecciones, builtins `cd`/`pwd`/`help`/`exit` |
| `/bin/soso-llm` | Inferencia LLM sobre modelos en `/models/` |
| `/bin/{ls,cat,echo,mkdir,rm,hexdump,halt}` | Coreutils |

`libsoso`: crt0, syscall wrappers, mini-libstd (256 KiB heap arena).

## Network & SSH

- smoltcp TCP/IPv4 + cliente DHCPv4 en kernel; fallback estático 10.0.2.15/24 si no hay lease en 8 s
- NIC polled (no IRQ-driven RX)
- sunset: curve25519 + ed25519 + chacha20-poly1305
- Auth: ed25519 public key only (`/etc/authorized_key`, 32 raw bytes)
- Host key: `/etc/ssh_host_key` (32-byte seed, persistent across mkfs)
- **Stack alignment:** `timer_isr` alinea rsp antes de `net::poll` (crypto SSE); los handlers `x86-interrupt` con código de error dejan `rsp%16==8` en los `call` — TODA llamada profunda desde esos handlers (mmap fault, kill_current, y el print del panic handler) pasa por el trampolín genérico `con_rsp_alineado` de interrupts.rs. Síntomas si se olvida: GPF esporádicos (error 0) en código con `movaps` y panics truncados
- **Reconexión:** `CloseWait`/`TimeWait` → `abort()` + `listen(22)`; mata shell huérfana

## sosomfs + LLM

- Segundo disco virtio-blk; montaje en `/models/<nombre>/`
- Ficheros `.som` con cabecera común (magic+crc+versión+payload_len, `pack_som`/`parse_som` en sosomodel): `manifest.som` (v2: GQA `num_kv_heads`, `rope_theta`, `rms_eps`), `index.som` (shape `[filas,columnas]` row-major, dtype F32/Q8_0), `tokenizer.som` (vocabulario SentencePiece-ish, opcional), shards `.tensor`
- Runtime (`soso-llm-core`): llama completo — RoPE, GQA, **Wo (`attn_output`)**, SwiGLU (`ffn_gate` opcional), `output_norm`, Q8_0 y Q4_K (layout GGML passthrough, matvec fusionado); pesos zero-copy (`TensorView` sobre mmap), KV cache f16, sampling temp/top-p (`sample.rs`), streaming (`generate_stream` + `StreamDecoder`); buffers reutilizados (`LayerScratch`) — libsoso libera solo bloques ≥1 MiB (mmap anónimo), no reservar por token
- **SIMD**: userspace compila con target propio `user/x86_64-soso-user.json` (SSE..AVX2+FMA, build-std); kernels AVX2 en `gemm.rs::avx2` con dispatch por `target_feature` (escalar = referencia para tests). **Estado FPU**: el kernel preserva x87/XMM/YMM con **xsave64** (`arch/fpu.rs`; fxsave NO basta — pierde las mitades altas YMM entre procesos): timer_isr guarda a `TIMER_FPU` antes de net::poll, `timer_tick` lo copia a `Process.fpu` al desalojar, `schedule_inner` restaura al reanudar, el page fault handler preserva en `mmap_fault_shim`; syscalls no preservan (los wrappers de libsoso llevan `clobber_abi("C")`). `init test` estresa YMM con dos hijos "fpu" concurrentes
- Harness rápido de calidad en host: `cargo run --release -p soso-llm-core --features std --example hostrun -- <modelo-dir> "<prompt>" <n>` (velocidad nativa, SOSO_DEBUG=1 para estadísticas por capa)
- `Runtime::validate_shapes()` comprueba index↔manifest antes de inferir
- Host: `cargo xtask convert-gguf` (GGUF llama F32/F16/Q8_0 → .som), `mkfs-sosomfs`, `mkmodel-soso` (tiny sintético, regenerado en cada mkfs)
- `SOSO_MODELS_DIR=<dir> cargo xtask run` empaqueta un modelo propio en vez de tiny
- Userspace: `soso-llm run <modelo> --prompt <texto>` vía mmap + greedy decode; mmap pagina bajo demanda (`handle_mmap_fault` — ojo: `map_page` toma `FRAME_ALLOC`, no llamarla con ese lock tomado)

## Coding constraints

1. **Minimize scope** — smallest correct diff; match existing style
2. **Kernel is `no_std`** — userspace uses `libsoso`, not std
3. **sosofs changes**: test on host first (`BlockDevice` over `File`)
4. **Do not break** `cargo xtask test` — it is the E2E gate
5. **QEMU fixed**: `-machine q35`, `-cpu max`, virtio PCI (not mmio)
6. **Spanish** for user-facing strings and docs in this repo

## Verification paths

| Layer | Command |
|-------|---------|
| sosofs unit + crash | `cargo test -p sosofs --features std` |
| Syscall regression | `/bin/init test` in QEMU |
| Full system | `cargo xtask test` |

## Out of scope (by design)

Multi-user, permissions, SFTP, IPv6, fork, signals, snapshots/compression in sosofs.
