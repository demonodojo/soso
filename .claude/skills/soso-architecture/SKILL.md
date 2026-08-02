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

`exit, read, write, open, close, seek, stat, getdents, mkdir, unlink, spawn, wait, sbrk, sleep_ms, halt, mmap, munmap, pipe, spawn_io, chdir, getcwd, meminfo` (+ GPU, TCP, hilos)

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
- **E/S de bloque agrupada (2026-08-02):** `BlockDevice::read_blocks(start, buf)` +
  `max_blocks_per_request()` (método por defecto = el bucle de siempre, así que
  los dispositivos de host no cambian); tope `sosomfs::MAX_REQ_BLOCKS = 32`
  (128 KiB), impuesto por el rebote DMA contiguo de virtio (`dma::alloc_pages`
  **panica** sin contigüidad y recicla por número exacto de páginas). Implementado
  en virtio, NVMe (MDTS del Identify + lista PRP + rebote persistente; antes 4 KiB
  por comando y `map_dma_uc` fugaba VA en cada uno) y live/USB
  (`read_sectors10`). Faltas de página: racimo de 128 KiB en
  `task/mod.rs::racimo`, con recorte parcial al final de región/fichero, buffer
  de tránsito estático (**no** frames contiguos: `allocate_contiguous` sólo
  avanza el cursor bump y trituraría el pool de 2 MiB) y caída al camino de
  4 KiB ante cualquier fallo. Los shards de sosomfs abren siempre `Fd::LazyFile`.
  Medido: `bench` 72 304 → 2 270 peticiones y 12,9 s → 0,7 s de disco;
  `tiny` 579 → 82 peticiones y 176 → 42 ms.
- **Contadores de E/S**: `kernel/src/drivers/blkstat.rs` (TSC, no PIT),
  `SYS_IOSTAT = 36` → `abi::IoStat`, comando `io` / `io reset` del kshell, líneas
  `soso-llm: disco —` y `carga en frío —`, eco en `bench-llm` y `xtask test`.
  **`uptime_ms()` subcuenta durante el polling de disco**: no juzgues E/S por
  tok/s, usa estos contadores o el reloj del host.
- **Staging asíncrono** (`user/soso-llm/src/staging.rs`): un hilo hace el
  prefetch de shards sobre el `MmapTensorSource` del hilo principal. Dos reglas
  que costó sangre aprender: (1) `SOURCE_PTR` se republica **en cada kick**, no
  una vez en `enable_worker` — el `StagedSource` se mueve dentro de
  `ModelBundle` justo después y un puntero crudo entregado a otro hilo no
  sobrevive a un move; (2) el hilo principal llama a `sincroniza()` antes de
  tocar `inner` en cualquier método del trait, porque la caché de shards es un
  `BTreeMap` sin sincronizar y dos hilos mutándolo lo dejan con punteros
  inventados. Síntoma de ambos: page fault de usuario en una dirección con
  pinta de cadena (p. ej. `0x6e7474612e3032be` = «…20.attn»). Coste: el
  solapamiento prefetch↔cómputo se pierde casi entero; recuperarlo pide un lock
  por shard dentro de `prefetch_shards_sync`, no quitar la sincronización.
- **Pendiente conocido**: la caché de bloques sigue siendo O(n) (`read_lba`
  busca linealmente sobre hasta 2048 entradas y `evict_lru` barre otra vez).
  Con las lecturas ya agrupadas es el siguiente cuello, y poblar la caché desde
  los racimos sin arreglarlo antes cuelga el banco de 128 MiB.
- Ficheros `.som` con cabecera común (magic+crc+versión+payload_len, `pack_som`/`parse_som` en sosomodel): `manifest.som` (v4: tabla `LayerSpec` por capa — `AttnKind` Gqa/Mla/Kda, `FfnKind` Dense/Moe/LatentMoe, overrides MLA/MoE; v3: MoE global `num_experts`, `num_experts_per_tok`, `moe_ffn_dim`; v2: GQA `num_kv_heads`, `rope_theta`, `rms_eps`; parse v1–v3 sintetiza layers uniformes), `index.som` (shape `[filas,columnas]` row-major, dtype F32/Q8_0/Q4_K), `tokenizer.som` (vocabulario SentencePiece-ish, opcional), shards `.tensor`
- **MoE (Mixtral-style, manifest v3):** `num_experts > 0` activa `forward_moe_ffn` en `layer.rs`: router `L{i}.ffn_gate_inp` → `topk_softmax` → SwiGLU por experto `L{i}.E{e}.ffn_{gate,up,down}` (un shard `.tensor` por tensor); prefetch de capa solo attn+router; expertos fríos vía `plan.rs::touch_moe_experts` + `source.prefetch_shards`; `keep_all_shards_after` retiene cache LRU de expertos calientes junto al working set de capas
- Runtime (`soso-llm-core`): llama denso o **MoE** — RoPE, GQA, **Wo (`attn_output`)**, SwiGLU (`ffn_gate` opcional o por experto), `output_norm`, Q8_0 y Q4_K (layout GGML passthrough, matvec fusionado); pesos zero-copy (`TensorView` sobre mmap), KV en `kv.rs` (f16 o int8 KIVI-lite), sampling temp/top-p (`sample.rs`), streaming (`generate_stream` / `generate_stream_planned` + `StreamDecoder`); buffers reutilizados (`LayerScratch`, incl. `router`/`moe_acc` en MoE) — libsoso libera solo bloques ≥1 MiB (mmap anónimo), no reservar por token
- **Planificador de recursos** (`plan.rs` + `ResourcePlanner`): lee `SYS_MEMINFO`, presupuesto de pesos (70 % libre+reclaimable), EWMA por capa/destino, replanifica cada 8 tokens; elige `KvDtype`, H2O y sparse según presión; **cache LRU de expertos MoE** (`touch_moe_experts`, stats `moe_hits`/`moe_misses`); **prefetch MoE especulativo** (`last_experts`, stats `moe_spec_hits`/`moe_spec_misses`); **staging AirLLM** (`stage.rs` kick/wait + `stage_wait_ms`); `Runtime::set_planner` recrea KV con el dtype; stats incluyen `kv_dtype_i8` / `h2o_enabled` / `sparse_attn` / PLD
- **KV cache** (`kv.rs` + `LayerKv`): `append` f16 o int8+escala/token; `load_k_head`/`load_v_head`; masa H2O; `slide_window_h2o(keep, sink, recent, …)`; decode vía `attention_decode_kv`
- **Atención** (`attn.rs`): decode FlashAttention-style tiled (tiles 64, path f16 clásico); `attention_decode_kv` (f16/I8 + masa); **Quest-lite** sparse si `seq > 256` (bloques 32, top-4 + sink/recent); **AVX2+FMA** f16→f32; prefetch shards stride 2 MiB
- **Prompt Lookup Decoding** (`prompt_lookup_draft_hinted`): greedy; hint de n autotuneado (`tune_pld` por tasa de aceptación) + fallback max→min; stats `pld_*` / `pld_prefer_n` / `pld_max_draft`
- **Hot path** (`LayerTiming`): EWMA matvec vs attn por capa (`observe_hotpath`); `soso-llm` imprime `hot path — matvec/attn ms/capa`. Decode: si KV f16 + denso + sin H2O → `attention_decode_f16_tiled` (SIMD); si no, `attention_decode_kv`. `LayerScratch.mass_buf` reutilizado (sin alloc por token)
- **Prefill**: `prefill_prompt` prefetch del embed N+1; residuales `add_f32`/`add_assign_f32` AVX2; SwiGLU helper
- **Reclaim kernel** (`mm/reclaim.rs`): clock (segunda oportunidad) sobre páginas mmap RO; marca de agua 4 MiB; TLB shootdown IPI (`0x42`) en lote — modelos > RAM degradan a I/O de disco
- **Optimizaciones paper → código** (mantener al día en cada etapa de `/loop` inferencia):

| Técnica | Origen | Módulo |
|--------|--------|--------|
| Layer streaming + release | LayerKV / FlexGen | `plan.rs`, `source.rs`, `runtime.rs` |
| Double-buffer staging (kick/wait) | AirLLM | `stage.rs`, `source.rs` kick/wait, `runtime.rs` |
| Prefetch MoE especulativo (hint token previo) | AirLLM | `plan.rs::last_experts`, `layer.rs` kick_moe |
| Streaming por experto (MoE) | AirLLM | `layer.rs::forward_moe_ffn`, `plan.rs::touch_moe_experts` |
| Prefetch layer-ahead / 2 MiB | ScoutAttention-style | `source.rs` |
| Ventana sink+recientes | StreamingLLM | `kv.rs::slide_window` |
| Eviction por masa attn | H2O | `kv.rs::slide_window_h2o`, masa en decode |
| KV int8 + escala/token | KIVI-lite | `kv.rs` `KvDtype::I8` |
| Attn sparse por bloques | Quest-lite | `attn.rs` `SPARSE_*` + planner |
| Online softmax tiled | FlashAttention decode | `attn.rs` |
| Draft n-gramo + autotune | Prompt Lookup Decoding | `attn.rs` hinted, `plan::tune_pld` |
| Perfil matvec vs attn | — (telemetría) | `LayerTiming`, `observe_hotpath` |
| Fast path attn f16 | Flash decode | `attention_decode_f16_tiled` si !H2O/!sparse |
| Capacidad KV pre-reservada | espíritu PagedAttention | `LayerKv::with_capacity_*` |
| Clock reclaim + shootdown | OS / vLLM-like | `kernel/src/mm/reclaim.rs` |

- **Cierre de etapa `/loop` (obligatorio):** al terminar cada pase de optimización, actualizar esta skill (tabla + bullets), espejo `.cursor/skills/soso-architecture/SKILL.md`, `soso-dev` si cambian tests/comandos, y `MANUAL-USUARIO.md` si hay strings o comportamiento visible al usuario. No dejar docs aplazados al “final del loop”.
- **SIMD**: userspace compila con target propio `user/x86_64-soso-user.json` (SSE..AVX2+FMA, build-std); kernels AVX2 en `gemm.rs::avx2` con dispatch por `target_feature` (escalar = referencia para tests). **Estado FPU**: el kernel preserva x87/XMM/YMM con **xsave64** (`arch/fpu.rs`; fxsave NO basta — pierde las mitades altas YMM entre procesos): timer_isr guarda a `TIMER_FPU` antes de net::poll, `timer_tick` lo copia a `Process.fpu` al desalojar, `schedule_inner` restaura al reanudar, el page fault handler preserva en `mmap_fault_shim`; syscalls no preservan (los wrappers de libsoso llevan `clobber_abi("C")`). `init test` estresa YMM con dos hijos "fpu" concurrentes
- Harness rápido de calidad en host: `cargo run --release -p soso-llm-core --features std --example hostrun -- <modelo-dir> "<prompt>" <n>` (velocidad nativa, SOSO_DEBUG=1 para estadísticas por capa)
- `Runtime::validate_shapes()` comprueba index↔manifest antes de inferir
- Host: `cargo xtask convert-gguf` (GGUF llama denso o MoE Mixtral → `.som` v4; trocea `ffn_*_exps` por experto), `mkfs-sosomfs` (multi-modelo: `mkfs-sosomfs dir1 dir2 imagen.img`), `mkmodel-soso` (`tiny` denso + `--moe` → `tiny-moe`; flags `--experts`, `--experts-per-tok`, `--moe-ffn`)
- Tests host MoE: `cargo test -p soso-llm-core --features std --test moe`
- `SOSO_MODELS_DIR=<dir> cargo xtask run` empaqueta un modelo propio en vez de tiny
- Userspace: `soso-llm run <modelo> --prompt <texto>` vía mmap + greedy decode; mmap pagina bajo demanda (`handle_mmap_fault` — ojo: `map_page` toma `FRAME_ALLOC`, no llamarla con ese lock tomado)

## Capa lxdde + GPU (L6, en curso)

- **lxdde** (`lxdde/`): capa DDE estilo `lx_emul` que compila C (drivers Linux
  portados o first-party) a `liblxdde.a` y lo enlaza al kernel Rust. Ports:
  `spike`, `testdrv`, `e1000e`, `nouveau`. Build: `cargo xtask lx-build <port>`
  lee `source.list`, compila con clang freestanding, y autogenera dummies
  (`lx_emul_trace_and_stop`) para símbolos undefined no provistos.
- **Port nouveau/nvkm** (`lxdde/ports/nouveau/`): bring-up de la GPU NVIDIA (GB205
  Blackwell y Ampere GA10x/**RTX 3060**, chip-aware). **62 fuentes nvkm/lib reales**
  de Linux 6.6.32 integradas (core, falcon, nvfw, ACR, mmu, fb, instmem, engine
  gr/fifo/dma base) vía shims mínimos en `lxdde/shim/include/` (slab/pci/mutex/... que
  cortan la avalancha de cabeceras arch del kernel). El **grafo de objetos nvkm real
  se construye en runtime** (`nvkm_bringup_lx.c` → `ga102_gsp_new`). El boot GSP
  efectivo y el compute siguen **soft/CPU** hasta cablear MMIO real (requiere G1/HW).
- **Syscalls GPU** (`soso-abi`): `SYS_GPU_INFO=17`, `SYS_GPU_ALLOC=18`,
  `SYS_GPU_MAP=19`, `SYS_GPU_SUBMIT=20`, `SYS_GPU_READ=33`, `SYS_GPU_FREE=34`,
  `SYS_MEMINFO=35` (frames totales/libres/reclaimable).
- **Puente Rust↔C**: `kernel/src/lxdde/gpu.rs` (`lx_nouveau_*`), drivers en
  `kernel/src/drivers/{gpu,nvidia_probe,nvidia_compute}.rs`. Modo por
  `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau`.
- Detalle completo (roadmap G1→G5 **GO** en GB205, VFIO/IOMMU, firmware, workflow de port nvkm):
  skill **`soso-gpu`**.

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
