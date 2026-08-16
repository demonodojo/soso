---
name: soso-dev
description: >-
  Build, run, test and debug the soso bare-metal OS in QEMU — cargo xtask,
  mkfs, SSH access, serial console, gdb and integration tests. Use when
  starting soso, compiling the kernel or userspace, running QEMU, connecting
  by SSH, troubleshooting boot/network, reading SOSOLOG.TXT from the live USB
  (`cargo xtask sosolog`), or running cargo xtask test.
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
| `cargo xtask build --drivers qemu` | Kernel mínimo (virtio-blk + virtio-net) |
| `cargo xtask fit-drivers target/SOSODRV.TXT` | Reempaqueta kernel según informe hwscan |
| `cargo xtask driver-add <git-url>` | Clona port lxdde externo a `lxdde/ports-extern/` |
| `cargo xtask run` | Build + launch QEMU (serial on stdio) |
| `cargo xtask gdb` | Frozen at boot; `gdb -ex 'target remote :1234'` |
| `cargo xtask mkfs` | Force-regenerate sosofs data image from `rootfs/` |
| `cargo xtask test` | Full integration: sosofs, boot, TCP, SSH, soso-llm, halt |
| `cargo xtask bench-llm` | Medir tok/s decode (modelo `bench`, SMP configurable) |
| `cargo xtask package-usb` | Artefactos clásicos (UEFI + data + models separados) |
| `cargo xtask package-usb-live` | Imagen live GPT única (`soso-live.img`, ver `docs/L5c-on-box.md`) |
| `cargo xtask flash-usb-live /dev/sdX --yes` | Graba live + estira p3 al sobrante del stick (p4 SOSOINSTALL 32 MiB al final) |
| `cargo xtask sosolog [/dev/sdX]` | Monta la ESP del USB live, imprime `SOSOLOG.TXT` y desmonta (`sudo` solo para mount) |
| `cargo xtask test-install` | Instalación nativa de punta a punta: 3 arranques OVMF (instalar por SSH → GPT del destino → `Boot####` del shim → arrancar solo del NVMe). Necesita `ovmf` y `sgdisk`; `SOSO_MODELS_SIZE=256M` para que sea rápido |
| `cargo xtask fetch-hf` | Descargar GGUF de Hugging Face, convertir a `.som` y preparar `SOSO_MODELS_DIR` |
| `cargo xtask convert-gguf` | Convert GGUF → `.som` layout (denso o MoE Mixtral, host tool) |
| `cargo run -p mkmodel-soso -- --moe target/tiny-moe-model` | Generar modelo sintético MoE (4 expertos, top-2) |
| `cargo run -p mkfs-sosomfs -- dir1 dir2 imagen.img --size 8G` | Empaquetar varios modelos en una imagen sosomfs |
| `cargo xtask lx-build` | Compilar `liblxdde.a` (drivers Linux portados) |
| `cargo xtask lx-build nouveau` | Compilar solo el port nouveau/nvkm (GPU, L6/G5 GO en GB205) |
| `cargo xtask lx-build iwlwifi` | Compilar driver Intel AX211 (WiFi + mac80211 mínimo) |
| `./scripts/l6-wifi-vfio-test.sh` | Passthrough VFIO WiFi AX211 a QEMU (prueba ALIVE) |
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

## WiFi (Intel AX211, hardware real)

```sh
cargo xtask lx-build iwlwifi
SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi cargo xtask build
SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi cargo xtask flash-usb-live /dev/sdX --yes
# VFIO passthrough a QEMU (requiere root + IOMMU):
sudo ./scripts/l6-wifi-vfio-test.sh
```

Config: `/etc/wifi.conf` (`ssid=`, `psk=`). Firmware en `rootfs/lib/firmware/iwlwifi-so-a0-gf-a0-*`.
Kshell: `wifi scan`, `wifi status`, `wifi connect <ssid> [psk]`, `hwscan` (informe PCI → serie y `SOSODRV.TXT` en live).

## Perfiles de drivers (`SOSO_DRIVERS` / `--drivers`)

| Preset | Features kernel | Uso |
|--------|-----------------|-----|
| `all` (default) | `drv-all` | Desarrollo y `cargo xtask test` |
| `qemu` | virtio-blk, virtio-net | Imagen mínima QEMU |
| `live-usb` | virtio + nvme + usb + live-disk | Pendrive live |

Tras arrancar en hardware con kernel mínimo, `hwscan` lista dispositivos PCI y
drivers ausentes. En host: `cargo xtask fit-drivers /media/.../SOSODRV.TXT`
(o `--esp /dev/sdX1`) regenera el kernel con los drivers necesarios.

Ports lxdde externos: repo con `source.list` (+ opcional `driver.toml`,
`firmware/`). `cargo xtask driver-add <url>` los registra en
`drivers-extern.toml` y `lx-build all` los incluye.

## Log del USB live (`cargo xtask sosolog`)

`SOSOLOG.TXT` está en la **ESP (partición 1, FAT)**. Linux no la monta sola
(oculta las EFI); el volumen que sí aparece suele ser p4 `SOSOINSTALL`, que
no tiene el log.

```sh
cargo xtask sosolog              # auto-detecta el USB live
cargo xtask sosolog /dev/sdX     # disco entero → p1
cargo xtask sosolog /dev/sdX1    # ESP concreta
cargo xtask sosolog | less
```

No uses `sudo cargo`: root no tiene rustup. La xtask pide `sudo` solo para
`mount`/`umount` (y monta con `uid`/`gid` del usuario para poder leer).
Implementación: `xtask/src/sosolog.rs`. El kernel vuelca el ring cada ~2 s
(`drivers/fatlog.rs`) sobre el hueco pre-creado en `package-usb-live`.

## What `run` does

1. Compiles userspace (`user/`) and copies ELFs to `rootfs/bin/`
2. Runs `mkfs-soso` on `rootfs/` → data disk image (64 MiB default)
3. Generates models disk (`target/soso-models.img`) with synthetic **tiny** (denso) and **tiny-moe** (MoE Mixtral-style) models
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

# Host: sosomfs import atómico (grow, crash sin commit, catálogo)
cargo test -q -p sosomfs --features std

# Host: soso-http (Range header; sin red real)
cargo test -q -p soso-http

# Host: soso-llm-core (planificador, kv KIVI/H2O, attn sparse, PLD, MoE), sosomodel, convert-gguf
cargo test -q -p soso-llm-core --features std -p sosomodel -p convert-gguf
# Subconjuntos útiles tras tocar inferencia:
#   cargo test -p soso-llm-core --features std -- plan:: kv:: attn:: moe::
# Hostrun MoE sintético:
#   cargo run -q --release -p mkmodel-soso -- --moe target/tiny-moe-model
#   cargo run --release -p soso-llm-core --features std --example hostrun -- target/tiny-moe-model @bos 4

# End-to-end (host tests ∥ build user/kernel, luego 4 shards QEMU en paralelo)
cargo xtask test

# QEMU: auto `-accel kvm` si /dev/kvm legible; forzar TCG para comparar:
SOSO_QEMU_ACCEL=tcg cargo xtask test

# Paralelismo QEMU (default 4 con KVM, 2 en TCG; 1 = secuencial para depurar)
SOSO_TEST_JOBS=1 cargo xtask test

# USB/xHCI: 4 escenarios en paralelo (mismo SOSO_TEST_JOBS, imágenes copiadas)
cargo xtask test-usb

# Logs por shard: target/test-{llm-dense,llm-moe,sys,reclaim}-serial.log
# Imágenes copiadas: target/test-{shard}-{bios,data,models}.img
# `cargo xtask run` y `bench-llm` siguen en puertos 2222/7777

# Los shards NO son intercambiables: cada uno existe por su configuración.
#   llm-dense  -smp 2   único sitio donde `ThreadPool` crea workers (`ncpu-1`);
#                       con 1 core ese código no se ejecuta. Incluye el paso
#                       «sigue viva tras dos pools de hilos».
#   reclaim    -m 96M   presión de memoria (`RECLAIM_MEM` en xtask/src/test.rs).
#                       Súbela si el bootloader falla con FrameAllocationFailed
#                       —el kernel ha crecido—, pero lo justo: con 72M la
#                       inferencia muere a media generación de forma inestable.
#   sys                 syscalls, pipes, SSH, `ask` (texto literal) y halt.

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

### Se ha colgado: ¿dónde? (monitor de QEMU)

Lo primero ante un cuelgue, antes de teorizar. Lanza QEMU con un monitor Unix y
pregúntale dónde está cada core:

```sh
# OJO: ruta corta, el socket UNIX tiene tope de 108 bytes (el scratchpad no cabe)
qemu-system-x86_64 … -monitor unix:/tmp/soso-mon.sock,server,nowait
```

```python
# info registers -a → un RIP por vCPU
s = socket.socket(socket.AF_UNIX); s.connect("/tmp/soso-mon.sock")
s.sendall(b"info registers -a\n")
```

| RIP | Dónde | Cómo resolverlo |
|-----|-------|-----------------|
| `0x4xxxxx` | userspace (ELF en 0x400000) | `objdump -d target/user/x86_64-soso-user/release/<bin>` |
| `0x100000xxxxx` | kernel (PIE en 0x10000000000) | `addr2line -f -C -e target/kernel/x86_64-soso/debug/kernel <rip - 0x10000000000>` |

Sano en reposo = todos en `enable_and_hlt`. Todos en el mismo RIP de usuario =
bucle de spin (así se cazó la fuga de workers de `ThreadPool`, 2026-08-16).

### Teclado PS/2 de verdad, sin pantalla

El banco entra siempre por SSH, así que **el camino teclado→tty no se ejercita
nunca** y ahí se escondieron dos cuelgues de placa. Se puede inyectar scancodes
reales (con su IRQ 1) por el mismo monitor:

```
sendkey a          # y spc, ret, minus, slash, dot…
```

Para reproducir el arranque live entero (rootfs por USB BOT, que es donde hay
contención de `HOSTS`), levanta QEMU a mano con la imagen live como
`usb-storage` sobre `qemu-xhci` y `-smp 8`; el `-drive` de `soso-bios.img` sigue
siendo el de arranque. Ver `xtask/src/test_install.rs` para el patrón de
argumentos.

## Skills layout

Skills live in `.claude/skills/`. `.cursor/skills` mirrors them — edit under `.claude/skills/`
and sync the mirror. Tras cada etapa de un `/loop` de inferencia/arquitectura: actualizar
`soso-architecture` (tabla paper→código), este skill si hay tests/comandos nuevos, y
`MANUAL-USUARIO.md` si el usuario ve strings o comportamiento distinto (skill
`soso-user-manual`).

## Common issues

| Symptom | Fix |
|---------|-----|
| SSH permission denied | Use `-i target/soso_test_key` or ensure `~/.ssh/id_ed25519.pub` existed before build |
| Connection refused :2222 | Wait for `sosh — escribe 'help'`; or prior QEMU still running → `pkill qemu-system-x86` |
| Teclado muerto tras la primera tecla (placa) | Algo del camino IRQ 1 toma un `lock()` o imprime; ver «Candados y contexto de interrupción» en `soso-architecture` |
| La máquina se arrastra tras usar `soso-llm`/`ask` | Workers del pool girando sin apagar; `ThreadPool` tiene que hacer `Drop` con `shutdown` + espera |
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
| No se ve `SOSOLOG.TXT` en el USB | Está en la ESP (p1), que Linux no monta; `cargo xtask sosolog` |
