---
name: soso-architecture
description: >-
  soso OS architecture — kernel layout, sosofs CoW filesystem, syscalls,
  userspace ABI, networking, SSH stack and coding constraints. Use when
  modifying kernel/, crates/, user/, xtask/, adding features, syscalls,
  drivers, or understanding how components interact. GPU NVIDIA: skill
  soso-gpu. WiFi iwlwifi: soso-wifi. Live USB, install and OTA: soso-live.
---

# soso — Architecture

Learning OS in Rust. **10/10 phases complete.** Bare-metal x86_64 on QEMU q35.

## Workspace layout

```
soso/
├── kernel/           # no_std kernel (x86_64-unknown-none, outside root workspace)
├── boot-shim/        # UEFI: BOOTMARK, buzón install/OTA, chainload bootsoso.efi
├── xtask/            # build image, mkfs, QEMU, live USB, integration tests
├── lxdde/            # DDE C: nouveau, iwlwifi, e1000e → liblxdde.a
├── crates/
│   ├── sosofs/       # CoW FS (no_std + "std" feature for host tests)
│   ├── sosomfs/      # model shards FS (VFS RO; import atómico SYS_SOM_*)
│   ├── soso-abi/     # syscall numbers, Stat, Dirent, errno
│   ├── gptdisk/      # GPT: leer/reubicar/reescribir tablas
│   ├── xhci-nostd/   # xHCI + USB BOT (live stick)
│   ├── soso-llm-core/  # inference runtime (host + userspace)
│   ├── sosomodel/    # .som manifest/index/shard format
│   ├── gguf2som/     # GGUF → .som (llama/MoE/Qwen/MLA)
│   ├── soso-http/    # HTTPS client (Range)
│   ├── soso-web-core/# HTML → texto / layout gráfico
│   ├── soso-audio/   # DSP captura / WAV
│   ├── soso-gpu/     # ABI GPU userspace
│   ├── soso-update-core/ # manifest, pack, mailbox OTA
│   └── block-dev/    # BlockDevice trait (virtio-blk, host File)
├── tools/
│   ├── mkfs-soso/    # rootfs dir → sosofs image + SSH keys
│   ├── mkfs-sosomfs/ # model tree → models disk image
│   ├── mkmodel-soso/ # modelos sintéticos tiny / tiny-moe / …
│   ├── convert-gguf/ # GGUF → .som
│   ├── convert-whisper/ # ggml Whisper → .som ASR
│   ├── cuda-proxy/   # L6-H TCP → llama-server
│   ├── gsp-hostcheck/ / iwl-hostcheck/  # parsers C en host
│   └── ssh-proto/    # host prototype (phase 9), isolated
├── user/             # libsoso, init, sosh, soso-llm/voz/web/hf/update, coreutils
└── rootfs/           # source tree embedded into disk by mkfs-soso
```

Skills de dominio: **`soso-dev`** (build/test), **`soso-gpu`**, **`soso-wifi`**,
**`soso-live`** (USB/install/OTA), **`soso-user-manual`**.

Docs operativos: [`docs/GUIA-OPERATIVA.md`](../../docs/GUIA-OPERATIVA.md),
[`docs/ESTADO.md`](../../docs/ESTADO.md), [`docs/HW-MATRIX.md`](../../docs/HW-MATRIX.md).

## Kernel (monolithic)

- Ring 0: drivers, FS, network, SSH — el kernel **no** se desaloja a sí mismo
- Ring 3: ELF, round-robin **preemptivo**; SMP userspace (`-smp N`, `ncpu` en
  `ThreadPool`). Con `-smp 1` no hay workers (`want == 0`)
- **spawn, not fork** — one PML4 per process, static ELFs at `0x400000`
- Entry: `syscall`/`sysret` (MSRs STAR/LSTAR)

### Syscalls principales

`exit, read, write, open, close, seek, stat, getdents, mkdir, unlink, spawn, wait, sbrk, sleep_ms, halt, mmap, munmap, pipe, spawn_io, chdir, getcwd, meminfo` (+ GPU, TCP, hilos, WiFi)

- **Instalación / OTA / ESP:** syscalls y huecos 8.3 — skill **`soso-live`** (`disk_*`, `bootreq_*`, `upd_*`, `espfat`)
- **Framebuffer / entrada:** `fb_info=62`, `fb_set_mode=63`, `fb_present=64`, `input_poll=65` (modo gráfico userspace; ratón PS/2 aux)
- **WiFi:** `wifi_scan=56`, `wifi_status=57`, `wifi_connect=58` — detalle en **`soso-wifi`**
- **Audio:** `audio_open=59`, `audio_read=60`, `audio_close=61` (HDA, `drv-hda`)

- **Pipes/redirecciones:** sosh usa `pipe` + `spawn_io`; hijos heredan cwd del padre
- **Señales (mínimo):** SIGINT/SIGKILL/SIGTERM vía `kill`; `kill(pid, 0)` sondea existencia (no entrega). Grupos con `setpgid`/`setsid`/`tcsetpgrp`; Ctrl-C (ISIG) al grupo en primer plano de la consola. Sin handlers (`sigaction`). El líder de sesión en el prompt recibe `-EINTR` en `read` en vez de morir
- **Escritura:** `open(O_WRONLY)` → buffer en kernel; `create_file` en sosofs al `close()`
- **Rutas:** `task/path.rs` resuelve relativas contra `Process.cwd` (default `/`)
- Sin permisos Unix

### Key subsystems

| Module | Role |
|--------|------|
| `arch/` | GDT/TSS, IDT, PIC+PIT 100 Hz, paging |
| `drivers/kbd.rs` | PS/2 + USB HID → cola tty; mapa **es** por defecto (`keymap.rs`), AltGr, teclas muertas |
| `drivers/fb.rs` | Consola GOP: buffer UTF-8 con glifos Latin-1 + €. Shadow en RAM → GOP **WC** (`arch/pat.rs` entrada 1 + `mm::set_write_combining`), copia `movntdq` + `sfence`, *jump scroll* de `rows/4` en ráfagas (<200 ms entre scrolls) con un solo `flush_all`. Ver «Consola GOP» abajo |
| `drivers/` | serial, pci, dma, registry; drivers opcionales vía features `drv-*` |
| `drivers/pci.rs` | ECAM + MSI-X. `devices()` = foto cacheada del bus (usar esta); `enumerate()` reescribe BARs, sólo en arranque |
| `drivers/espfat.rs` | Ficheros 8.3 contiguos en la ESP live (SOSOLOG, SOSODRV, SOSOBOOT, SOSOWIFI, SOSOUPD, SOSOKRN, SOSOKRN.MET) |
| `xtask/src/sosolog.rs` | Host: monta la ESP del USB, imprime `SOSOLOG.TXT` y desmonta (`cargo xtask sosolog`) |
| `fs/` | sosofs (blk0) + sosomfs (blk1); VFS enruta `/models/*` |
| `vfs.rs` | Router: lectura/escritura sosofs; modelos → sosomfs (read-only) |
| `net/` | smoltcp, DHCPv4 al arrancar (fallback 10.0.2.15), polled from scheduler |
| `net/ssh.rs` | sunset SSH-2, una sesión, CRLF en tx_push, reset_socket al desconectar |
| `kshell.rs` | Emergency kernel-shell (`soso>`): `help`, `dmesg [save]`, `hwscan`, `wifi`, `io`, `halt`, … |
| `task/` | Processes (cwd, console), scheduler, syscall, path normalization |

## sosofs (v1)

- 4 KiB blocks, little-endian, full CoW B+ tree (no journal)
- Dual superblocks A/B — atomic commit via generation increment
- **`grow_to`** al montar si la partición/imagen es mayor que `block_count`
  (QEMU sparse, pendrive). En live/instalado GPT: **`soso-resize rootfs +N`**
  mueve margen libre de modelos (`SYS_FS_RESIZE`).
- CRC32C checksums on all nodes and file extents
- **Host-first development**: `cargo test -p sosofs --features std` with crash-injection before kernel integration
- Commits on write `close()` and every ~2 s

## Userspace

| Binary | Role |
|--------|------|
| `/bin/init` | PID 1: spawns sosh, relaunches on crash; `init test` = syscall regression suite |
| `/bin/sosh` | Shell: pipes, redirecciones, builtins `cd`/`pwd`/`help`/`exit`/`wifi`/`ask` |
| `/bin/soso-llm` | Inferencia LLM sobre modelos en `/models/`; subcomando `ask` (texto crudo, silencioso, REPL) |
| `/bin/soso-hf` | Descarga GGUF desde Hugging Face Hub → import atómico a `/models/` (`pull`/`search`/`list`) |
| `/bin/ask-modelo` | Fija el modelo de `ask` en `/etc/llm.conf` |
| `/bin/soso-install` | Instalador nativo desde el live: lista discos y uso, elige destino, guardas por tipo de partición, clon, `gptdisk::relayout` + GUID nuevos, y petición de entrada UEFI |
| `/bin/soso-resize` | Amplía sosofs robando margen libre al final de modelos (`SYS_FS_RESIZE`; live/instalado GPT) |
| `/bin/soso-update` | Releases GitHub: rootfs por fichero (sin rollback de binarios; progreso en `/etc/actualiza.estado`); kernel vía `SOSOUPD.TXT` + `SOSOKRN.BIN` + meta `SOSOKRN.MET` (recovery verificable) |
| `/bin/soso-web` | Navegador mínimo: HTTPS + HTML→texto (modo lectura) o framebuffer (modo `--grafico`) |
| `/bin/{ls,cat,echo,mkdir,rm,hexdump,halt}` | Coreutils |

`libsoso`: crt0, syscall wrappers, mini-libstd (256 KiB heap arena), `linea::Lector`
(lectura de línea con eco: **acepta UTF-8** y borra por carácter; lee **byte a byte**
para que lo que venga detrás de la línea se quede en la cola de la tty y lo vea el
hijo que se acabe de lanzar).

**`ask`** (`user/soso-llm/src/ask.rs`, cliente en `user/sosh/src/main.rs`): `sosh` lo
resuelve **antes de tokenizar** y habla por TCP con el demonio de máquina
`soso-llm askd` en `127.0.0.1:7420` — es la única forma de que comillas, tildes y
`|`/`>` lleguen al modelo, porque el tokenizador de la shell no tiene escapes.
El askd carga el modelo en la **primera pregunta** y lo mantiene entre consola, SSH
y reconexiones; sólo recarga al cambiar de modelo, si `refresh_mem` lo exige, o al
`halt`. Protocolo: línea de pregunta → chunks de texto → byte `0xFF` (fin). Un
generate a la vez; el listen sigue aceptando. `:eco` va local sin askd. Arranque
perezoso: el cliente conecta y, si falla, `spawn_io(..., FD_SERIAL_TTY)` sin
`wait` — el askd queda atado a la consola serie (y a `SOSOLOG.TXT`), no a la
sesión SSH de quien lo lanzó. **`spawn_io` → SOSA:** el kernel escribe argv
completo en la pila; el crt0 de libsoso entrega a `main(&str)` solo `argv[1..]`
(argv[0] = path del binario). Si no, askd/vozd ven `/bin/soso-llm askd` e
imprimen el usage en vez de arrancar el demonio. Kernel:
`tcp_connect(127.0.0.1:port)` empareja con un listener userspace sin
NIC loopback (`kernel/src/net/loopback.rs`). `soso-llm run` no usa askd (carga en
frío). Config en `/etc/llm.conf`, que **no fija modelo por defecto**: se
usa el primero de `/models`, y el empaquetado live (`package-usb-live` /
`flash-usb-live`) pone el modelo demo delante de `tiny` sintético — sin
pendrive `package-usb-live` usa **qwen3.8-27b**; al flashear elige el mejor
GGUF que quepa (tinyllama → mistral-7b → qwen3.8-27b en 32 GB+). Las imágenes
de prueba QEMU siguen con `synthetic tiny`. Fijar un nombre ahí lo hereda
toda imagen que se genere, y avisa en cada respuesta si no viaja
con ella — por eso `ask-modelo` escribe el fichero en el disco de la máquina, no en el
árbol. `:eco <texto>` se resuelve antes de leer nada: devuelve el texto tal cual llegó
y es lo que hace verificable el camino crudo (`ask :eco a|b>c "x"`).
**askd usa `ThreadPool`** (ncpu-1) en cada generate y lo suelta al terminar;
los workers duermen en futex entre matvecs (si giran, `Drop` no vuelve al
`accept` y Mixtral en un core parece colgado). Staging async sigue apagado
en askd (con SMP el worker no pone `done`). `Runtime::layer_hook` emite un
punto por capa para que el cliente no corte a los 4 min de silencio. Si no hay
pool de VRAM (`pool VRAM=no`), Mixtral va a CPU: avisa y sugiere `:modelo tiny`.

**Voz / ASR** (`user/soso-voz/`, cliente en `user/sosh`): demonio `vozd` en
`127.0.0.1:7421` (molde de askd). Protocolo: `:escucha [ms]`, `:wav <ruta>`,
`:modelo`, `:modelos` → stream terminado en `0xFF`. Runtime en
`soso-llm-core/src/asr.rs`; DSP en `crates/soso-audio`. Captura por
`SYS_AUDIO_*` (59–61) y driver Intel HDA (`drv-hda`, `kernel/src/drivers/hda.rs`).
Config `/etc/voz.conf`. **Nunca autoejecutar**: sosh inserta la transcripción en
la línea; Enter confirma. F4 = push-to-talk (byte `0x12` en `linea.rs`).
Modelo ASR en sosomfs (`tiny-asr` sintético o `SOSO_ASR_DIR` / `convert-whisper`).

**Navegador soso-web** (`user/soso-web/`, parser en `crates/soso-web-core/`):
modo lectura (HTTPS + reflow HTML a texto en consola/SSH) y modo `--grafico`
(framebuffer + ratón). Sin JavaScript. Cliente HTTPS vía `soso-http` +
`TcpTransport` sobre syscalls de red. Modo lectura: `soso-web <url>` o
`soso-web --local /ruta.html`; navegación `<n>` enlace, `u URL`, `b`, `q`.
Modo gráfico: `soso-web --grafico <url>` (syscalls FB + `input_poll`, TTF en
`/lib/fonts/DejaVuSans.ttf` vía fontdue, `--fuente` para otra ruta). Prueba local: `/etc/web-prueba.html`. Tests host: `cargo test -p soso-web-core`;
E2E: paso `soso-web: HTML local` en `cargo xtask test`.

**Cargar un segundo modelo en el mismo proceso** destapó que `StagingWorker::spawned`
era por instancia: el hilo de staging y `STAGE` son del proceso, así que arrancaba un
segundo worker y reseteaba `generation` (que el vivo leía como kick) → dos hilos sobre
el mismo `BTreeMap`. Ahora el flag es global (`WORKER_VIVO`).

## Network & SSH

- smoltcp TCP/IPv4 + cliente DHCPv4 en kernel; fallback estático 10.0.2.15/24 solo con virtio-net/e1000e (no en WiFi ni en rtl8169 de placa)
- **Loopback userspace:** `tcp_connect(127.0.0.1:port)` → par de búferes kernel contra un `tcp_listen` del mismo puerto (sin paquetes ni iface loopback); multi-accept; `EADDRINUSE` / `ECONNREFUSED`. Cerrar un extremo = EOF en el otro (`tcp_is_connected` mira `peer_closed`); si no, `read_timeout` devolvía EAGAIN para siempre y sosh se quedaba colgada cuando askd moría. Cerrar un extremo = EOF en el otro (`tcp_is_connected` mira `peer_closed`); si no, `read_timeout` devolvía EAGAIN para siempre y sosh se quedaba colgada cuando askd moría
- **WiFi (lxdde/iwlwifi):** Intel AX211 (`8086:7f70/51f0/54f0`, context-info gen3, firmware `so-a0-gf-a0` + pnvm) y AX200 (`8086:2723`, context-info gen2, firmware `cc-a0`, sin pnvm); driver first-party en `lxdde/ports/iwlwifi/` (TLV fw, MVM scan/assoc/TX); mini-supplicant WPA2 EAPOL en `net/wifi_wpa.rs`; backend `NicDev::LxWifi` (prioridad si ya está asociado; si no, ethernet primero); credenciales `SOSOWIFI.TXT` (ESP live) o `/etc/wifi.conf`; DHCP tras asociación (sin fallback slirp); sosh `wifi scan|status|connect`; kshell `wifi scan|status|connect`
- **Ethernet Realtek:** `drv-rtl8169` nativo (`drivers/rtl8169.rs`) según Linux `r8169`; PCI `10ec:8168/8161/8162/8167/8136`; anillos DMA + PHY (PHYAR o GPHY OCP); backend `NicDev::Rtl8169`; DHCP real, sin 10.0.2.x
- **Live USB:** perfil `live-usb` = virtio + nvme + usb + live-disk + **e1000e** + **rtl8169** (Realtek `10ec:8168`, Linux r8169) + nouveau + iwlwifi (`SOSO_LXDDE_MODE=nouveau,iwlwifi`); SSH :22 tras lease DHCP. El `e1000e` nativo se inicializa salvo que el puerto lxdde `e1000e` lleve ese mismo chip (`if !modes.e1000e` en main.rs) — gatearlo también por `modes.iwlwifi`, como estaba, dejaba la ethernet muerta en el live, donde el iwlwifi siempre está activo
- **hwscan:** `registry.rs` emite **una línea por dispositivo PCI**, con driver o sin él: `drv: <bdf> <vid>:<did> <driver> <compilado|ausente|desconocido> clase <cc>:<ss>:<pi> <texto>`, y destaca al final cada controlador de red (clase `02`) que nadie reclama (`hwscan: RED SIN DRIVER …`). Se imprime en **todos** los arranques; antes sólo salía si faltaba un driver *conocido*, o sea que el hardware sin regla —el que hay que diagnosticar— no dejaba rastro. Los cuatro primeros campos son contrato: `xtask::drivers::parse_hwscan_line` los lee por posición
- **Drivers modulares:** features Cargo `drv-virtio-blk`, `drv-virtio-net`, `drv-e1000e`, `drv-rtl8169`, `drv-nvme`, `drv-usb`, `drv-gpu-nvidia`, `drv-live-disk`; meta `drv-all` (default). Metadatos PCI en `drivers/registry.rs`; `hwscan` en kshell; informe en `SOSODRV.TXT` (ESP live). Host: `SOSO_DRIVERS=qemu|live-usb|all` o `--drivers`; `cargo xtask fit-drivers <informe>` reempaqueta; `cargo xtask driver-add <git-url>` registra ports lxdde externos en `lxdde/ports-extern/`
- **Enganche de la pila:** `net::attach_now()` sondea backends y monta smoltcp; `net::try_attach()` es la versión limitada a un intento/segundo que llama `poll()`. La distinción importa: `poll()` entra por cada vuelta del bucle ocioso y por cada tick, y mientras no haya NIC el sondeo se repetía entero — en placa real, sin virtio-net, eso era una reenumeración del ECAM y un «net: no se encontró ningún virtio-net» por vuelta, que parecía un cuelgue del live (2026-08-31). Las sondas de dispositivo (`virtio_net::init`) cachean su resultado con `Once`
- NIC polled (no IRQ-driven RX)
- sunset: curve25519 + ed25519 + chacha20-poly1305
- Auth: ed25519 public key only (`/etc/authorized_key`, 32 raw bytes)
- Host key: `/etc/ssh_host_key` (32-byte seed, persistent across mkfs)
- **Stack alignment:** `timer_isr` alinea rsp antes de `net::poll` (crypto SSE); los handlers `x86-interrupt` con código de error dejan `rsp%16==8` en los `call` — TODA llamada profunda desde esos handlers (mmap fault, kill_current, y el print del panic handler) pasa por el trampolín genérico `con_rsp_alineado` de interrupts.rs. Síntomas si se olvida: GPF esporádicos (error 0) en código con `movaps` y panics truncados
- **Reconexión:** `CloseWait`/`TimeWait` → `abort()` + `listen(22)`; mata shell huérfana

## sosomfs + LLM

- Segundo disco virtio-blk; montaje en `/models/<nombre>/`
- **VFS read-only:** `create`/`append`/`mkdir` en `/models/*` siguen rechazados; el
  guest **no** escribe la partición con `SYS_DISK_WRITE` (QEMU: blk1 fuera de
  `raw_disk`; live USB: disco de arranque → `EBUSY`).
- **Import atómico (kernel):** syscalls `SYS_SOM_BEGIN` / `PUT` / `COMMIT` /
  `ABORT` + scratch `SYS_SOM_SCRATCH_*` (`kernel/src/som_import.rs`,
  `crates/sosomfs/src/import.rs`). Una sesión; extents primero, catálogo +
  `generation++` al commit; recarga del catálogo en RAM tras commit. **Grow al
  montar:** si la partición GPT es mayor que `sb.total_blocks`, actualiza el
  superbloque (`grow_models_if_needed` en `kernel/src/fs.rs`).
- **`soso-hf pull`:** Hub → GGUF (HTTP Range o scratch en cola p3) → `gguf2som` →
  import → `/models/<nombre>/` visible sin reiniciar.
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
- Ficheros `.som` con cabecera común (magic+crc+versión+payload_len, `pack_som`/`parse_som` en sosomodel): `manifest.som` (v4: tabla `LayerSpec` por capa — `AttnKind` Gqa/Mla/Kda/Gated/Gdn, `FfnKind` Dense/Moe/LatentMoe, overrides MLA/MoE/Qwen; v3: MoE global `num_experts`, `num_experts_per_tok`, `moe_ffn_dim`; v2: GQA `num_kv_heads`, `rope_theta`, `rms_eps`; parse v1–v3 sintetiza layers uniformes), `index.som` (shape `[filas,columnas]` row-major, dtype F32/Q8_0/Q4_K), `tokenizer.som` (vocabulario SentencePiece-ish, opcional), shards `.tensor`
- **MoE (Mixtral-style, manifest v3):** `num_experts > 0` activa `forward_moe_ffn` en `layer.rs`: router `L{i}.ffn_gate_inp` → `topk_softmax` → SwiGLU por experto `L{i}.E{e}.ffn_{gate,up,down}` (un shard `.tensor` por tensor); prefetch de capa solo attn+router; expertos fríos vía `plan.rs::touch_moe_experts` + `source.prefetch_shards`; `keep_all_shards_after` retiene cache LRU de expertos calientes junto al working set de capas
- Runtime (`soso-llm-core`): llama denso o **MoE** — RoPE, GQA, **Wo (`attn_output`)**, SwiGLU (`ffn_gate` opcional o por experto), `output_norm`, Q8_0 y Q4_K (layout GGML passthrough, matvec fusionado); pesos zero-copy (`TensorView` sobre mmap), KV en `kv.rs` (f16 o int8 KIVI-lite), sampling temp/top-p (`sample.rs`), streaming (`generate_stream` / `generate_stream_planned` + `StreamDecoder`); buffers reutilizados (`LayerScratch`, incl. `router`/`moe_acc` en MoE) — libsoso libera solo bloques ≥1 MiB (mmap anónimo), no reservar por token
- **Planificador de recursos** (`plan.rs` + `ResourcePlanner`): lee `SYS_MEMINFO`, presupuesto de pesos (70 % libre+reclaimable), **split trunk-first** (tronco pin+anillo antes que caché MoE; presets `auto|tight|balanced|max-pin`), clasificación genérica `trunk|routed_expert|always_resident`, EWMA por capa/destino, replanifica cada 8 tokens; elige `KvDtype`, H2O y sparse según presión; **pin prefix + anillo 1–2 slots** (`pinned_layers`, `ring_slots`); **cache LRU de expertos MoE** con telemetría resident vs JIT; **prefetch MoE especulativo** (`last_experts`); **staging AirLLM** (`stage.rs` kick/wait + `stage_wait_ms`); **feedback I/O-bound** (`io_bound` si `stage_wait` ≫ matvec+attn → preferir remoto, mantener anillo=2; cap double-buffer si 2×capa > RAM libre → `prefetch_single_buffered`; embed oversized → gather-only `embed_gather_only`); stats `trunk_hits/misses`, `trunk_bytes_read`, `moe_resident_hits`, `moe_jit_hits`; **offload GPU trunk-first + pool MoE** (atención/router/FFN denso/Sxx pinneados por capa; expertos `Lxx.Eyy` en pool LRU de VRAM con tamaño on-device Q4_K/Q8_0, no ×8 f32)
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
| Gated attn + Gated DeltaNet (Qwen3.5/3.8) | Qwen3.8 | `arch.rs` `forward_gated_attn`/`forward_gdn_attn`, `AttnKind::Gated/Gdn`, `gguf2som` arch `qwen35`/`qwen38`; KV recurrente O(1) en `LayerKv::gdn_s` |
| Shard cache lock (staging ∥ compute) | — | `source.rs::CacheLock` (TOCTOU-safe insert), `staging.rs` wait en release |
| Offload GPU trunk-first + pool MoE | — | `plan.rs` pack `TRUNK_GPU_PROJ` + `gpu_experts`, VRAM Q4_K crudo |

- **Cierre de etapa `/loop` (obligatorio):** al terminar cada pase, actualizar el skill de dominio (esta tabla si es inferencia; `soso-gpu`/`soso-wifi`/`soso-live` si tocan esos stacks), `soso-dev` si cambian tests/comandos, y `MANUAL-USUARIO.md` si hay strings o UX visible. Mantener en sync `.claude/skills/`, `.cursor/skills/` y `.agents/skills/`. No dejar docs aplazados al “final del loop”.
- **SIMD**: userspace compila con target propio `user/x86_64-soso-user.json` (SSE..AVX2+FMA, build-std); kernels AVX2 en `gemm.rs::avx2` con dispatch por `target_feature` (escalar = referencia para tests). **Estado FPU**: el kernel preserva x87/XMM/YMM con **xsave64** (`arch/fpu.rs`; fxsave NO basta — pierde las mitades altas YMM entre procesos): timer_isr guarda a `TIMER_FPU` antes de net::poll, `timer_tick` lo copia a `Process.fpu` al desalojar, `schedule_inner` restaura al reanudar, `irq::dispatch` preserva en `net_poll_shim` si bomba la red al salir a ring 3 (no usa `TIMER_FPU`), el page fault handler preserva en `mmap_fault_shim`; syscalls no preservan (los wrappers de libsoso llevan `clobber_abi("C")`). `init test` estresa YMM con dos hijos "fpu" concurrentes
- Harness rápido de calidad en host: `cargo run --release -p soso-llm-core --features std --example hostrun -- <modelo-dir> "<prompt>" <n>` (velocidad nativa, SOSO_DEBUG=1 para estadísticas por capa)
- `Runtime::validate_shapes()` comprueba index↔manifest antes de inferir
- Host: `cargo xtask convert-gguf` (GGUF **llama**, **deepseek2** MLA o **qwen35/qwen38** → `.som` v4; `--pack-trunk` empaqueta attn+FFN por capa; trocea `ffn_*_exps` por experto; `ffn_*_shexp` → `Sxx` con `num_shared_experts`; Qwen: capas `attn_q` → Gated, el resto GDN; GGUF `blk.N.post_attention_norm` → `Lxx.ffn_norm`, aborta si falta), `mkfs-sosomfs` (multi-modelo: `mkfs-sosomfs dir1 dir2 … imagen.img`), `mkmodel-soso` (`tiny` denso + `--moe` → `tiny-moe` + `--attn mla` → `tiny-mla` + `--moe --ffn-kind latent-moe` → `tiny-latent-moe` en imagen por defecto; flags `--attn mla|kda`, `--ffn-kind latent-moe`, `--shared-experts`, `--pack-trunk`, …). Offload GPU userspace: F32/Q8_0/Q4_K/**MXFP4** dequant-on-upload en `user/soso-llm/src/gpu.rs`
- Tests host arquitecturas: `cargo test -p soso-llm-core --features std --test arch_ext` (MLA/LatentMoE/shared/MXFP4/Gated/GDN)
- Tests host MoE: `cargo test -p soso-llm-core --features std --test moe`
- `SOSO_MODELS_DIR=<dir> cargo xtask run` empaqueta un modelo propio en vez de tiny
- Userspace: `soso-llm run <modelo> --prompt <texto>` vía mmap + greedy decode; mmap pagina bajo demanda (`handle_mmap_fault` — ojo: `map_page` toma `FRAME_ALLOC`, no llamarla con ese lock tomado)

## Capa lxdde + GPU (L6) + WiFi

- **lxdde** (`lxdde/`): capa DDE estilo `lx_emul` que compila C a `liblxdde.a`
  y lo enlaza al kernel Rust. Ports: `spike`, `testdrv`, `e1000e`, `nouveau`,
  **`iwlwifi`**. Build: `cargo xtask lx-build <port>` lee `source.list`, clang
  freestanding, dummies `lx_emul_trace_and_stop` para undefined no provistos.
  WiFi: skill **`soso-wifi`**. Live USB / xHCI / OTA: **`soso-live`**.
- **Port nouveau/nvkm** (`lxdde/ports/nouveau/`): bring-up de la GPU NVIDIA (GB205
  Blackwell y Ampere GA10x/**RTX 3060**, chip-aware). **62 fuentes nvkm/lib reales**
  de Linux 6.6.32 integradas (core, falcon, nvfw, ACR, mmu, fb, instmem, engine
  gr/fifo/dma base) vía shims mínimos en `lxdde/shim/include/` (slab/pci/mutex/... que
  cortan la avalancha de cabeceras arch del kernel). El **grafo de objetos nvkm real
  se construye en runtime** (`nvkm_bringup_lx.c` → `ga102_gsp_new`). El boot GSP
  efectivo y el compute en GB205 van por la ruta lx-native (`fmc_lx`/`gsp_*`).
- **Syscalls GPU** (`soso-abi`): `SYS_GPU_INFO=17`, `SYS_GPU_ALLOC=18`,
  `SYS_GPU_MAP=19`, `SYS_GPU_SUBMIT=20`, `SYS_GPU_READ=33`, `SYS_GPU_FREE=34`,
  `SYS_MEMINFO=35` (frames totales/libres/reclaimable).
- **Puente Rust↔C**: `kernel/src/lxdde/gpu.rs` (`lx_nouveau_*`), drivers en
  `kernel/src/drivers/{gpu,nvidia_probe,nvidia_compute}.rs`. Modo por
  `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau`.
- Detalle completo (roadmap G1→G5 **GO** en GB205, VFIO/IOMMU, firmware, workflow de port nvkm):
  skill **`soso-gpu`**.

## Instalación nativa, OTA y USB

Detalle (particiones, ESP 8.3, shim, TRB 17 bits, buzón `SOSOUPD`, meta
`SOSOKRN.MET`): skill **`soso-live`**. Límites publicados: [`docs/ESTADO.md`](../../docs/ESTADO.md).
Resumen: `soso-install` clona + `gptdisk::relayout` + GUID nuevos; NVRAM la toca
el shim. Kernel OTA: backup en `SOSOKRN.BIN`, fases durable en `SOSOKRN.MET`
(staged/backup/applying/probando); init confirma `OK` tras rootfs + `/tmp/sosh-ready`
con `pid=<pid>` de esa instancia de sosh (prefault + write/close ok).
Rootfs OTA: parcial por hash, reintento vía `/etc/actualiza.estado`, sin rollback
automático de binarios viejos. Transferencias USB: Normal TRB 17 bits → **no enviar 128 KiB en un TRB**
(`mass_storage` trocea a 64 KiB). Bounce xHCI persistente. Tests:
`test-install`, `test-update` (incl. recovery simulado), `test-usb`, host
`cargo test -p soso-update-core --features std --tests`. `VERSION` → `/etc/soso-release`.

## Hilos de usuario: join por futex

`thread_spawn` crea **procesos** del scheduler que comparten el `AddrSpace`.
Al salir, el kernel escribe `1` en la palabra `join_uaddr` y hace `futex_wake`
(`libsoso::thread::JoinHandle::join`). Los hilos ya no pasan por `wait()` del
padre. `ThreadPool::Drop` sigue poniendo `shutdown` y esperando `vivos`.
Regla: cualquier cosa que lance hilos los apaga antes de morir.

## Self-hosting (ruta A)

Ver [`docs/SELF-HOSTING.md`](../../docs/SELF-HOSTING.md). Syscalls 70–82,
`sosofs` v11 (`NAME_MAX` 255), `soso-ed`, `soso-forja`, `soso-std`, scaffolding
`config/rust-soso/` + `tools/sosoas` + `tools/wild-soso`. Forja remota: el
cliente exige HTTP 2xx; el servidor lee cuerpos binarios por `Content-Length`
y construye en `target/forja-work` (no en el checkout). `local` solo planifica;
`build-local` copia `/var/forja-out`.

## Estado FPU y excepciones de CPU

`xrstor` **carga MXCSR siempre** desde la imagen si la máscara incluye SSE/AVX,
ignore lo que ignore `XSTATE_BV`: un `FpuArea` a ceros deja **MXCSR=0** = las seis
excepciones SIMD desenmascaradas, y la primera operación inexacta levanta #XM.
`FpuArea::inicial()` escribe MXCSR=0x1F80 y MXCSR_MASK=0xFFBF a mano
(`arch/fpu.rs`). Los `FpuArea::empty()` que quedan son buffers de save→restore,
donde el contenido inicial da igual.

La IDT instala **todas** las excepciones 0..31 que soso puede ver, no solo
#UD/#GP/#PF/#DF (`instalar_excepciones_restantes` en `arch/interrupts.rs`): con la
IDT incompleta, cualquier excepción sin entrada se convierte en un `double fault`
mudo y se pierde el diagnóstico. Pasó dos veces —la IRQ1 del teclado y #XM— y en
las dos costó una sesión entera.

**QEMU no entrega #XM**: esta clase de fallo solo sale en silicio. `init test`
comprueba el *registro* con `stmxcsr` en vez de esperar la excepción.

## Candados y contexto de interrupción

`PROCS`, `HOSTS` (usb_storage) y la consola son `spin::Mutex` **no reentrantes**, y
`with_current`/`with_fd` toman `PROCS` con las interrupciones ABIERTAS. Regla: todo lo
que corra dentro de un handler de IRQ usa `try_lock`, nunca `lock`, y **no imprime**.
Se saltó tres veces en el camino de la IRQ 1 del teclado y la placa se clavaba en la
primera tecla (2026-08-16): `kick_if_tty_waiting` (PROCS), `usb_storage::poll_keyboard_scancode`
(HOSTS — el mismo candado que tiene cogido cualquier lectura del disco live, o el flush
de `fatlog` cada 2 s) y `log_scancode_raw` (consola). **QEMU no lo ve**: las pruebas
entran por SSH y la IRQ 1 nunca se dispara. Los scancodes de diagnóstico se leen ahora
con `kbd` en la kernel-shell.

## Consola GOP: el coste es el tipo de memoria, no el rasterizado

El bootloader mapea el framebuffer con PTEs limpios (PAT 0 = WB) y la MTRR de
la apertura de la GPU en placa es **UC**: cada store es una transacción de bus
serializada. En la ROG (GOP AMD `1002:1638`, 1920×1080×4 = 8 MiB por pantalla,
sin serie) un scroll eran dos millones de stores `u32` UC ≈ 100 ms, y las trazas
de arranque se arrastraban línea a línea (2026-09-11). Tres capas, en orden de
impacto:

1. **PAT WC** (`arch/pat.rs`): sólo la entrada 1 (PWT=1) pasa de WT a WC —
   nadie la usaba, e `ensure_mmio_mapped`/`map_dma_uc` (índice 3 = UC) siguen
   igual. PAT WC + MTRR UC → WC en Intel y AMD (es lo que hace `ioremap_wc` en
   Linux). BSP en `fb::init`, cada AP en `ap_entry` (el SDM exige el mismo PAT
   en todos los cores). `mm::set_write_combining(va, len)` cambia sólo los bits
   de tipo de los PTEs ya mapeados (4 KiB o 2 MiB) y hace `wbinvd`. Log:
   `fb: fís=0x… WC N páginas`; si dice `sin WC`, la CPU no anuncia PAT.
2. **`copy_to_gop`**: alinea el destino a 16 a mano y va con `movntdq`
   (`_mm_stream_si128`) + `sfence` al cerrar cada `flush_rect`. Un `memcpy`
   normal mete `movaps` y en el GOP AMD era #GP por alineación (el cuelgue en
   el primer scroll antes de `fatlog`); el `u32` volátil que lo sustituyó era
   correcto pero lento.
3. **Jump scroll** (`filas_salto`): si el scroll anterior fue hace <200 ms
   (`RAFAGA_NS`, ráfaga: arranque, `ls`, `dmesg`) se saltan `rows/4` filas y
   las siguientes líneas sólo pintan su fila; interactivo sigue de una en una.
   `sync_rows_after_scroll` hace el memmove, rasteriza sólo las filas con
   texto y vuelca **una vez** (`flush_all`), no banda + fila a fila. Cada
   `write` sigue volcándose en el acto: el último `boot:` visible sigue
   acotando dónde se colgó el arranque. QEMU no mide nada de esto (KVM ignora
   el tipo de memoria del guest); la prueba visual es `cargo xtask fb-shot`.

## PCI: enumerar el bus no es una lectura pasiva

`pci::enumerate()` mide cada BAR0 por el método estándar — escribe `0xffff_ffff`,
lee la máscara y restaura (`bar_size`, drivers/pci.rs). Durante ese instante el
BAR decodifica en una dirección falsa: con las colas de un dispositivo ya en
marcha, cualquier MMIO o DMA en vuelo cae en el hueco. En `pci::init()` es
inocuo (nadie ha programado nada); **después de levantar los drivers, no**.

Regla: todo lo que sólo quiera *mirar* el bus usa **`pci::devices()`**, la foto
que `init()` toma una vez y cachea. El hwscan lee de ahí. Se aprendió haciendo
que `print_hwscan()` corriera en todos los arranques (2026-08-31): cuatro
barridos con virtio-blk y virtio-net vivos colgaban el shard `llm-dense` de la
suite a los pocos minutos, con la máquina respondiendo al ping pero no al disco.
Quedan seis llamantes de `enumerate()` post-arranque (usb_storage, nvme, gpu,
nvidia_probe, e1000e, lxdde/pci): corren en secuencia durante el boot y sin
tráfico, pero son la misma clase de bug.

## Coding constraints

1. **Minimize scope** — smallest correct diff; match existing style
2. **Kernel is `no_std`** — userspace uses `libsoso`, not std
3. **sosofs changes**: test on host first (`BlockDevice` over `File`)
4. **Do not break** `cargo xtask test` ni `cargo xtask check` — son la puerta E2E/CI
5. **QEMU fixed**: `-machine q35`, `-cpu max`, virtio PCI (not mmio)
6. **Spanish** for user-facing strings and docs in this repo

## Verification paths

| Layer | Command |
|-------|---------|
| All host checks + builds | `cargo xtask check` |
| sosofs unit + crash | `cargo test -p sosofs --features std` |
| GPT del instalador | `cargo test -p gptdisk` (sin features) |
| OTA recovery (host) | `cargo test -p soso-update-core --features std --tests` |
| Syscall regression | `/bin/init test` in QEMU |
| Full system | `cargo xtask test` (debe quedar en verde; sin fallos «conocidos» en ask) |
| Instalación nativa live→disco | `cargo xtask test-install` (3 arranques OVMF) — **`soso-live`** |
| OTA E2E | `cargo xtask test-update` (apply + recovery + manifiesto inválido) |
| USB/xHCI | `cargo xtask test-usb` |
| Matriz hardware A8 | Tras SOSOLOG de placa: `parse-logs` (`gb205-dgpu`, `ax211-wifi`); no `ok` sin evidencia. `hw-matrix show`. **`soso-dev`** |
| Teclado/tty, hilos con SMP | Sólo con `-smp >1` y `sendkey` por el monitor de QEMU; ver `soso-dev` → Debugging |
| Parser firmware iwl | `./scripts/l6-iwl-fw-hostcheck.sh` |

## Out of scope (by design)

Multi-user, permissions, SFTP, IPv6, fork, sigaction/handlers, snapshots/compression in sosofs.
