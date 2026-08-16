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
│   ├── gptdisk/      # GPT: leer/reubicar/reescribir tablas (no_std + tests host)
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

- **Instalación:** `disk_list=37`, `disk_read=38`, `disk_write=39` (raw, 512 B/LBA; `raw_disk` solo deja escribir NVMe y nunca el disco de arranque), `bootreq_write=41` / `bootreq_read=42` (único camino a la ESP del disco live, y solo al fichero pre-creado `SOSOBOOT.TXT`)

- **Pipes/redirecciones:** sosh usa `pipe` + `spawn_io`; hijos heredan cwd del padre
- **Escritura:** `open(O_WRONLY)` → buffer en kernel; `create_file` en sosofs al `close()`
- **Rutas:** `task/path.rs` resuelve relativas contra `Process.cwd` (default `/`)
- Sin signals ni permisos Unix

### Key subsystems

| Module | Role |
|--------|------|
| `arch/` | GDT/TSS, IDT, PIC+PIT 100 Hz, paging |
| `drivers/` | serial, pci, dma, registry; drivers opcionales vía features `drv-*` |
| `drivers/espfat.rs` | Localiza ficheros 8.3 contiguos en la ESP del live; lo comparten `fatlog` (SOSOLOG), `drvlog` (SOSODRV) y `bootreq` (SOSOBOOT) |
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
| `/bin/sosh` | Shell: pipes, redirecciones, builtins `cd`/`pwd`/`help`/`exit`/`ask` |
| `/bin/soso-llm` | Inferencia LLM sobre modelos en `/models/`; subcomando `ask` (texto crudo, silencioso, REPL) |
| `/bin/ask-modelo` | Fija el modelo de `ask` en `/etc/llm.conf` |
| `/bin/soso-install` | Instalador nativo desde el live: guardas por tipo de partición, clon, `gptdisk::relayout` + GUID nuevos, y petición de entrada UEFI |
| `/bin/{ls,cat,echo,mkdir,rm,hexdump,halt}` | Coreutils |

`libsoso`: crt0, syscall wrappers, mini-libstd (256 KiB heap arena), `linea::Lector`
(lectura de línea con eco: **acepta UTF-8** y borra por carácter; lee **byte a byte**
para que lo que venga detrás de la línea se quede en la cola de la tty y lo vea el
hijo que se acabe de lanzar).

**`ask`** (`user/soso-llm/src/ask.rs`): `sosh` lo resuelve **antes de tokenizar** y
lanza `/bin/soso-llm ask <texto crudo>` — es la única forma de que comillas, tildes y
`|`/`>` lleguen al modelo, porque el tokenizador de la shell no tiene escapes.
`soso-llm` lo despacha sobre su `args` sin trocear. `run_model` está partido en
`preparar_sesion` + `generar` (`verboso` apaga el diagnóstico) para que el REPL cargue
el modelo una vez. Config en `/etc/llm.conf`, fallback al primer modelo de `/models`.
**Cargar un segundo modelo en el mismo proceso** destapó que `StagingWorker::spawned`
era por instancia: el hilo de staging y `STAGE` son del proceso, así que arrancaba un
segundo worker y reseteaba `generation` (que el vivo leía como kick) → dos hilos sobre
el mismo `BTreeMap`. Ahora el flag es global (`WORKER_VIVO`).

## Network & SSH

- smoltcp TCP/IPv4 + cliente DHCPv4 en kernel; fallback estático 10.0.2.15/24 si no hay lease en 8 s
- **WiFi (lxdde/iwlwifi):** Intel AX211 (`8086:7f70`); transporte Gen2 + firmware en `/lib/firmware/`; mini-supplicant WPA2 en `net/wifi_wpa.rs`; backend `NicDev::LxWifi` si no hay Ethernet; config `/etc/wifi.conf`; kshell `wifi scan|status|connect`
- **Drivers modulares:** features Cargo `drv-*` + `drv-all` (default); `drivers/registry.rs`; kshell `hwscan`; `SOSODRV.TXT` en ESP live; host `SOSO_DRIVERS`, `cargo xtask fit-drivers`, `cargo xtask driver-add`
- **Drivers modulares:** features Cargo `drv-virtio-blk`, `drv-virtio-net`, `drv-e1000e`, `drv-nvme`, `drv-usb`, `drv-gpu-nvidia`, `drv-live-disk`; meta `drv-all` (default). Metadatos PCI en `drivers/registry.rs`; `hwscan` en kshell; informe en `SOSODRV.TXT` (ESP live). Host: `SOSO_DRIVERS=qemu|live-usb|all` o `--drivers`; `cargo xtask fit-drivers <informe>` reempaqueta; `cargo xtask driver-add <git-url>` registra ports lxdde externos en `lxdde/ports-extern/`
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
  sobrevive a un move; (2) `sincroniza()` solo en `release_shards_except`;
  `load_*`/`tensor_view` usan `CacheLock` en `source.rs` (insert con
  double-check). El worker solo hace `prefetch_shards_sync`. Síntoma de puntero
  muerto: page fault con pinta de cadena (p. ej. `…20.attn`). Host:
  `ThreadStagedSource` con hilo std.
- **Caché de bloques O(1)** (`sosomfs/src/cache.rs`): índice abierto
  `lba → entrada` con hash de Fibonacci (los LBA de un shard son consecutivos y
  el módulo directo los amontona) y **lápidas** para las bajas; recompacta sólo
  al pasar de 1/4 de tabla. Evicción por reloj de segunda oportunidad, con la
  misma semántica que el LRU exacto al que sustituye (nunca `PIN`; `STREAM` sólo
  desaloja `STREAM`, y `None` antes que robarle el sitio a otra política). Con
  esto el racimo puede poblar la caché (`read_range_cacheado`), que sin el
  índice colgaba el banco de 128 MiB. Medido en `tiny`: 82 → **46 peticiones**,
  1986 → **1122 bloques**, 42 → **25 ms**.
  - **Cómo se prueba** (`crates/sosomfs/tests/cache.rs`, 9 tests): con cachés
    diminutas para que haya desalojo real, y comprobando invariantes en cada
    paso. Aviso ganado a pulso: **medir aciertos no vale**. `buscar` verifica
    `entries[v].lba == lba`, así que una ranura obsoleta da la respuesta
    correcta, y una tabla saturada sigue acertando barriéndola entera. La
    propiedad que hay que afirmar es el **número de sondeos** (`sondeos()`,
    sólo en host). Validado por sabotaje: anular `desindexar` dispara el test
    con «10 aciertos han costado 225 sondeos».
- Ficheros `.som` con cabecera común (magic+crc+versión+payload_len, `pack_som`/`parse_som` en sosomodel): `manifest.som` (v4: tabla `LayerSpec` por capa — `AttnKind` Gqa/Mla/Kda, `FfnKind` Dense/Moe/LatentMoe, overrides MLA/MoE; v3: MoE global `num_experts`, `num_experts_per_tok`, `moe_ffn_dim`; v2: GQA `num_kv_heads`, `rope_theta`, `rms_eps`; parse v1–v3 sintetiza layers uniformes), `index.som` (shape `[filas,columnas]` row-major, dtype F32/Q8_0/Q4_K), `tokenizer.som` (vocabulario SentencePiece-ish, opcional), shards `.tensor`
- **MoE (Mixtral-style, manifest v3):** `num_experts > 0` activa `forward_moe_ffn` en `layer.rs`: router `L{i}.ffn_gate_inp` → `topk_softmax` → SwiGLU por experto `L{i}.E{e}.ffn_{gate,up,down}` (un shard `.tensor` por tensor); prefetch de capa solo attn+router; expertos fríos vía `plan.rs::touch_moe_experts` + `source.prefetch_shards`; `keep_all_shards_after` retiene cache LRU de expertos calientes junto al working set de capas
- Runtime (`soso-llm-core`): llama denso o **MoE** — RoPE, GQA, **Wo (`attn_output`)**, SwiGLU (`ffn_gate` opcional o por experto), `output_norm`, Q8_0 y Q4_K (layout GGML passthrough, matvec fusionado); pesos zero-copy (`TensorView` sobre mmap), KV en `kv.rs` (f16 o int8 KIVI-lite), sampling temp/top-p (`sample.rs`), streaming (`generate_stream` / `generate_stream_planned` + `StreamDecoder`); buffers reutilizados (`LayerScratch`, incl. `router`/`moe_acc` en MoE) — libsoso libera solo bloques ≥1 MiB (mmap anónimo), no reservar por token
- **Planificador de recursos** (`plan.rs` + `ResourcePlanner`): lee `SYS_MEMINFO`, presupuesto de pesos (70 % libre+reclaimable), **split trunk-first** (tronco pin+anillo antes que caché MoE; presets `auto|tight|balanced|max-pin`), clasificación genérica `trunk|routed_expert|always_resident`, EWMA por capa/destino, replanifica cada 8 tokens; elige `KvDtype`, H2O y sparse según presión; **pin prefix + anillo 1–2 slots** (`pinned_layers`, `ring_slots`); **cache LRU de expertos MoE** con telemetría resident vs JIT; **prefetch MoE especulativo** (`last_experts`); **staging AirLLM** (`stage.rs` kick/wait + `stage_wait_ms`); stats `trunk_hits/misses`, `trunk_bytes_read`, `moe_resident_hits`, `moe_jit_hits`
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
| Trunk-first split (pin antes que expert cache) | kimi-k3-in-c | `plan.rs::compute_trunk_first_split`, CLI `--mem-*` |
| Pin prefix + ring streaming | kimi-k3-in-c trunk | `plan.rs::keep_shards_after`, `pinned_layers` |
| Packed trunk por capa (`Lxx.trunk.tensor`) | kimi-k3-in-c pack | `gguf2som --pack-trunk`, offsets en `index.som` |
| True-resident hits (tronco/MoE) | kimi-k3-in-c telemetría | `plan.rs` stats, líneas `soso-llm` |
| Arquitecturas MLA/KDA/LatentMoE/shared/MXFP4 | Kimi K3 | `arch.rs`, `manifest.rs` v4, `mkmodel-soso --attn/--ffn-kind/--shared-experts`; MLA cache latente en `kv.rs` + `attention_decode_mla_latent` |
| Shard cache lock (staging ∥ compute) | — | `source.rs::CacheLock` (TOCTOU-safe insert), `staging.rs` wait en release |
| Sim cache MoE offline | kimi-k3 sim_cache | `tools/sim-moe-cache.py`, `SOSO_MOE_TRACE=1` hostrun |

- **Cierre de etapa `/loop` (obligatorio):** al terminar cada pase de optimización, actualizar esta skill (tabla + bullets), espejo `.cursor/skills/soso-architecture/SKILL.md`, `soso-dev` si cambian tests/comandos, y `MANUAL-USUARIO.md` si hay strings o comportamiento visible al usuario. No dejar docs aplazados al “final del loop”.
- **SIMD**: userspace compila con target propio `user/x86_64-soso-user.json` (SSE..AVX2+FMA, build-std); kernels AVX2 en `gemm.rs::avx2` con dispatch por `target_feature` (escalar = referencia para tests). **Estado FPU**: el kernel preserva x87/XMM/YMM con **xsave64** (`arch/fpu.rs`; fxsave NO basta — pierde las mitades altas YMM entre procesos): timer_isr guarda a `TIMER_FPU` antes de net::poll, `timer_tick` lo copia a `Process.fpu` al desalojar, `schedule_inner` restaura al reanudar, el page fault handler preserva en `mmap_fault_shim`; syscalls no preservan (los wrappers de libsoso llevan `clobber_abi("C")`). `init test` estresa YMM con dos hijos "fpu" concurrentes
- Harness rápido de calidad en host: `cargo run --release -p soso-llm-core --features std --example hostrun -- <modelo-dir> "<prompt>" <n>` (velocidad nativa, SOSO_DEBUG=1 para estadísticas por capa)
- `Runtime::validate_shapes()` comprueba index↔manifest antes de inferir
- Host: `cargo xtask convert-gguf` (GGUF **llama** o **deepseek2** MLA → `.som` v4; `--pack-trunk` empaqueta attn+FFN por capa; trocea `ffn_*_exps` por experto; `ffn_*_shexp` → `Sxx` con `num_shared_experts`), `mkfs-sosomfs` (multi-modelo: `mkfs-sosomfs dir1 dir2 … imagen.img`), `mkmodel-soso` (`tiny` denso + `--moe` → `tiny-moe` + `--attn mla` → `tiny-mla` + `--moe --ffn-kind latent-moe` → `tiny-latent-moe` en imagen por defecto; flags `--attn mla|kda`, `--ffn-kind latent-moe`, `--shared-experts`, `--pack-trunk`, …). Offload GPU userspace: F32/Q8_0/Q4_K/**MXFP4** dequant-on-upload en `user/soso-llm/src/gpu.rs`
- Tests host arquitecturas: `cargo test -p soso-llm-core --features std --test arch_ext` (MLA/LatentMoE/shared/MXFP4)
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

## Instalación nativa (live → disco)

`soso-install` (userspace) clona el pendrive al NVMe destino, repara su GPT con
`crates/gptdisk` (respaldo al final del disco real, última partición estirada,
GUID regenerados — si no, el destino sería indistinguible del USB para el
firmware) y deja `INSTALL <guid-ESP>` en `SOSOBOOT.TXT` de la ESP del live vía
`SYS_BOOTREQ_WRITE`. **El kernel no puede tocar la NVRAM**: `bootloader_api::BootInfo`
no expone la System Table y tras `ExitBootServices` no hay Runtime Services. Quien
crea el `Boot####` es `boot-shim/src/bootentry.rs`, en el arranque siguiente del USB;
deja `DONE Boot#### soso` en el mismo fichero y nunca aborta el arranque si falla.

Transferencias: `raw_disk::{read,write}` van en bloques de 128 KiB (`MAX_XFER`) y
`mass_storage` trocea a 64 KiB — **el campo de longitud de un Normal TRB es de 17
bits, así que 0x20000 exactos se desbordan a cero** y el endpoint acaba en Stall
(`CSW inválido sig=0`). El rebote DMA de xHCI es persistente (`XhciController::bounce`):
el asignador DMA del kernel no libera, y uno por comando tiraba a la basura tanta
memoria como datos movidos.

Verificación: `cargo xtask test-install` (3 arranques OVMF, incluye un NVMe falso con
swap/ESP que el instalador debe rechazar).

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
