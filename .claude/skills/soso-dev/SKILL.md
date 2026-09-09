---
name: soso-dev
description: >-
  Build, run, test and debug the soso bare-metal OS in QEMU — cargo xtask,
  mkfs, SSH access, serial console, gdb and integration tests. Use when
  starting soso, compiling the kernel or userspace, running QEMU, connecting
  by SSH, troubleshooting boot/network, reading SOSOLOG.TXT or the hwscan
  report SOSODRV.TXT from the live USB (udisksctl on ESP p1; `cargo xtask sosolog` needs sudo/TTY),
  `cargo xtask check`, `cargo xtask hw-matrix`, or running cargo xtask test.
---

# soso — Development workflow

Minimalist Rust OS (x86_64 bare-metal) running in QEMU q35. Monousuario.

Guía operativa: [`docs/GUIA-OPERATIVA.md`](../../docs/GUIA-OPERATIVA.md).
Estado y matriz hardware: [`docs/ESTADO.md`](../../docs/ESTADO.md), [`docs/HW-MATRIX.md`](../../docs/HW-MATRIX.md).

## Requirements

- **rustup** with nightly from `rust-toolchain.toml` (bootloader 0.11 needs `-Zbuild-std`)
- **qemu-system-x86_64** (`sudo apt install qemu-system-x86`)
- **OpenSSH client** + **ssh-keygen** (for SSH tests and access)

## Commands (from repo root)

| Command | Action |
|---------|--------|
| `cargo xtask build` | Compile kernel → `target/soso-bios.img` |
| `cargo xtask check` | Pre-commit/CI: host tests, builds, iwl/GSP hostchecks, hw-matrix parser |
| `cargo xtask build --drivers qemu` | Kernel mínimo (virtio-blk + virtio-net) |
| `cargo xtask fit-drivers target/SOSODRV.TXT` | Reempaqueta kernel según informe hwscan |
| `cargo xtask driver-add <git-url>` | Clona port lxdde externo a `lxdde/ports-extern/` |
| `cargo xtask run` | Build + launch QEMU (serial on stdio) |
| `cargo xtask gdb` | Frozen at boot; `gdb -ex 'target remote :1234'` |
| `cargo xtask mkfs` | Force-regenerate sosofs data image from `rootfs/` |
| `cargo xtask test` | Full integration: sosofs, boot, TCP, SSH, soso-llm, halt |
| `cargo xtask bench-llm` | Medir tok/s decode (modelo `bench`, SMP configurable) |
| `cargo xtask package-usb` | Artefactos clásicos (UEFI + data + models separados) |
| `cargo xtask package-usb-live` | Imagen live GPT única (`soso-live.img`, modelo demo **qwen3.8-27b**; ver `docs/L5c-on-box.md`) |
| `cargo xtask flash-usb-live /dev/sdX --yes` | Mide el stick, empaqueta el mejor modelo GGUF que quepa, graba live y estira p3. p4 `SOSOINSTALL` (FAT) va en la imagen tras el rootfs para que Linux la monte. `SOSO_LIVE_OFFLINE=1`: sin HF; el mayor ya en `target/*-model/` que quepa. **`--skip-models`**: solo ESP+rootfs (bucle diario); **`--only kernel|rootfs`**. **El agente no puede ejecutarlo:** sudo pide contraseña y no hay TTY; deja el comando al usuario (skill **soso-live**). |
| `cargo xtask sosolog [/dev/sdX]` | Monta la ESP, imprime `SOSOLOG.TXT` y desmonta. **Pide sudo/TTY:** el agente no lo lanza; usa `udisksctl` (skill **soso-live**) |
| `cargo xtask sosolog --drv [/dev/sdX]` | Igual con `SOSODRV.TXT` (hwscan). Mismo límite de sudo; el agente lee el fichero montando p1 con udisks |
| `cargo xtask test-install` | Instalación nativa de punta a punta: 3 arranques OVMF (instalar por SSH → GPT del destino → `Boot####` del shim → arrancar solo del NVMe). Necesita `ovmf` y `sgdisk`; `SOSO_MODELS_SIZE=256M` para que sea rápido |
| `cargo xtask test-update` | OTA E2E OVMF: apply, corte simulado + recovery, manifiesto inválido |
| `cargo xtask test-resize` | B1: host + grow/recovery QEMU live (`drv-live-disk`, tiny 256 MiB; no uses el preset `qemu`) |
| `cargo xtask hw-matrix show` | Matriz validación hardware (A8); `init`, `collect`, `record-boot` |
| `./scripts/l6-a8-collect.sh` | Recoger boot/bench en placa → `docs/hw-matrix.json`. Tras SOSOLOG de placa: `parse-logs` a `gb205-dgpu` y `ax211-wifi`; no `--boot-ok` ni etapas `ok` sin sosh / `UCODE_ALIVE_NTFY` / GSP RPC. |
| `cargo xtask release [--publish]` | Empaqueta release en `target/release-soso/v<VERSION>/`; `--publish` sube a GitHub Releases |
| `cargo xtask fetch-hf` | Descargar GGUF de Hugging Face, convertir a `.som` y preparar `SOSO_MODELS_DIR` |
| `cargo xtask fetch-whisper` | Descargar `ggml-tiny.bin` (curl reanudable) y convertir a `target/whisper-tiny-model` |
| `cargo xtask convert-gguf` | Convert GGUF → `.som` layout (denso o MoE Mixtral, host tool) |
| `cargo run -p mkmodel-soso -- --moe target/tiny-moe-model` | Generar modelo sintético MoE (4 expertos, top-2) |
| `cargo run -p mkfs-sosomfs -- dir1 dir2 imagen.img --size 8G` | Empaquetar varios modelos en una imagen sosomfs |
| `cargo xtask lx-build` | Compilar `liblxdde.a` (drivers Linux portados) |
| `cargo xtask lx-build nouveau` | Compilar solo el port nouveau/nvkm (GPU, L6/G5 GO en GB205) |
| `cargo xtask lx-build iwlwifi` | Compilar driver Intel AX211/AX200 (WiFi + mac80211 mínimo) |
| `./scripts/l6-iwl-fw-hostcheck.sh` | Parser TLV iwlwifi vs ucode del rootfs (sin NIC) |
| `./scripts/l6-wifi-vfio-test.sh` | Passthrough VFIO WiFi AX211 a QEMU (prueba ALIVE) |
| `cargo xtask test-distributed-llm` | Humo LLM repartido (2 QEMU); `-3` = tres nodos |
| `cargo xtask install-disk /dev/nvmeXn1 --yes` | Dual-boot desde Linux: escribe live + entrada GRUB |
| `cargo xtask sosomfs-check` | Comprobar imagen/árbol sosomfs en host |
| `cargo xtask g1-check` | Checklist host G1 (IOMMU/VFIO, firmware, BAR0) |
| `cargo xtask g3-check` | Checklist bring-up GSP (firmware, módulos, fases) |
| `./scripts/l6-pack-firmware.sh` | Empaquetar firmware GSP gb205 (.zst→.bin) en rootfs |
| `./scripts/l6-g3-gsp-hostcheck.sh` | GSP hostcheck (~1 s, G4d–G4f encoders, sin GPU) |
| `./scripts/l6-g1-vfio-test.sh` | Ciclo VFIO completo (cap PCIe Gen3 antes; ver soso-gpu) |
| `./scripts/l6-h-start-cuda.sh` | L6-H nativo (requiere `llama-server` en PATH) |

**Exit QEMU:** `Ctrl-A X` (not Ctrl-C).

> **GPU / L6 (NVIDIA nouveau/GSP):** G1→G5 **GO** en GB205 — skill **`soso-gpu`**.
> **WiFi Intel:** AX211/AX200, hostcheck y VFIO — skill **`soso-wifi`**.
> **Live USB / install / OTA:** particiones, ESP, shim — skill **`soso-live`**.
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

## WiFi (Intel AX211/AX200, hardware real)

```sh
# Live USB ya incluye nouveau+iwlwifi; el usuario flashea (el agente no:
# sudo pide contraseña y no hay TTY). Compila y deja este comando:
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only kernel
# Primer flash completo / sin tocar modelos:
# sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models
# WiFi: edita SOSOWIFI.TXT en ESP p1 o /etc/wifi.conf antes de flashear
# SSH en placa: ssh -i target/soso_test_key soso@<ip>  (puerto 22)
./scripts/l6-iwl-fw-hostcheck.sh          # parser TLV, sin hardware
sudo ./scripts/l6-wifi-vfio-test.sh       # VFIO AX211 en QEMU
```

Config: `SOSOWIFI.TXT` (ESP) o `/etc/wifi.conf` (`ssid=`, `psk=`). Firmware en `rootfs/lib/firmware/iwlwifi-so-a0-gf-a0-*` (AX211) y `iwlwifi-cc-a0-*.ucode` (AX200).
sosh: `wifi scan`, `wifi status`, `wifi connect <ssid> [psk]`.
Kshell: `wifi scan`, `wifi status`, `wifi connect <ssid> [psk]`, `hwscan` (informe PCI completo → serie y `SOSODRV.TXT` en live; lee la foto cacheada del bus, no lo reenumera).

## Perfiles de drivers (`SOSO_DRIVERS` / `--drivers`)

| Preset | Features kernel | Uso |
|--------|-----------------|-----|
| `all` (default) | `drv-all` | Desarrollo y `cargo xtask test` |
| `qemu` | virtio-blk, virtio-net | Imagen mínima QEMU |
| `live-usb` | virtio + **e1000e** + **rtl8169** + nvme + usb + live-disk + nouveau + iwlwifi | Pendrive live (ethernet Intel/Realtek + GPU + WiFi) |

Tras arrancar en hardware, `hwscan` lista **todos** los dispositivos PCI —con
driver o sin él— y destaca al final los controladores de red que nadie reclama:

```
hwscan: 8 dispositivos PCI, 3 con driver conocido
drv: 00:04.0 1af4:1000 virtio-net compilado clase 02:00:00 ethernet
drv: 00:1f.0 8086:2918 sin-driver desconocido clase 06:01:00 puente
hwscan: RED SIN DRIVER 00:1f.6 8086:15fc (ethernet)
```

Sale por serie en cada arranque y, en live, a `SOSODRV.TXT` (el agente lo lee
montando p1 con `udisksctl`; skill **soso-live**). En host: `cargo xtask fit-drivers
/media/.../SOSODRV.TXT` (o `--esp /dev/sdX1`) regenera el kernel con los drivers
necesarios. Si tu NIC sale como `sin-driver`, ese `VVVV:DDDD` es lo que decide si
basta con ampliar la lista de IDs de `drivers/registry.rs` (el `e1000e` nativo
sólo cubre `8086:10d3/100e/10f5/10a4`, las tarjetas de QEMU; `rtl8169` cubre
Realtek `10ec:8168/8161/8162/8167/8136`) o hay que portar uno.

Ports lxdde externos: repo con `source.list` (+ opcional `driver.toml`,
`firmware/`). `cargo xtask driver-add <url>` los registra en
`drivers-extern.toml` y `lx-build all` los incluye.

## Log del USB live (ESP p1)

`SOSOLOG.TXT` / `SOSODRV.TXT` están en la **ESP (partición 1, FAT)**. Linux no
la monta sola (oculta las EFI); el volumen que sí aparece es p4 `SOSOINSTALL`,
que **no** tiene el log.

**Agente:** no lances `cargo xtask sosolog` (pide `sudo mount`, no hay TTY).
Monta p1 con udisks, lee, desmonta — receta completa en skill **soso-live**
(sección *ESP en el host*):

```sh
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTTYPENAME,RM   # p1 vfat/EFI, RM=1; no p4
findmnt -n -o TARGET /dev/sda1                          # ¿ya montada?
udisksctl mount -b /dev/sda1 --no-user-interaction
ESP=$(findmnt -n -o TARGET /dev/sda1)
cat "$ESP/SOSOLOG.TXT"
cat "$ESP/SOSODRV.TXT"
udisksctl unmount -b /dev/sda1 --no-user-interaction
```

**Usuario** (tiene TTY): `cargo xtask sosolog [--drv] [/dev/sdX]`. Pide sudo
solo para `mount`/`umount`; no `sudo cargo` (root no tiene rustup).
Implementación: `xtask/src/sosolog.rs`. El kernel vuelca el ring cada ~2 s
(`drivers/fatlog.rs`) sobre el hueco pre-creado en `package-usb-live`.

## What `run` does

1. Compiles userspace (`user/`) and copies ELFs to `rootfs/bin/`
2. Runs `mkfs-soso` on `rootfs/` → data disk image (empaquetado compacto; QEMU
   amplía sparse a `SOSO_ROOTFS_SIZE`, default 32 GiB, y sosofs crece al montar)
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

# Host: parser HTML/reflow de soso-web (sin red)
cargo test -q -p soso-web-core

# Host: Forja (HTTP fragmentado, cuerpo >4 KiB, binario, 400/401/500/503, timeout)
cargo test -q -p soso-forja-server -- --test-threads=1

# Host: parser hw-matrix (fixtures negativos B5)
cargo test -q -p xtask hw_matrix::

# Host: resize + caché GPT (B1/B6; lookup no relee la tabla)
cargo test -q -p soso-resize-core

# Host: soso-llm-core (planificador, kv KIVI/H2O, attn sparse, PLD, MoE, Qwen Gated/GDN), sosomodel, convert-gguf
cargo test -q -p soso-llm-core --features std -p sosomodel -p convert-gguf
# Subconjuntos útiles tras tocar inferencia:
#   cargo test -p soso-llm-core --features std -- plan:: kv:: attn:: moe::
#   cargo test -p soso-llm-core --features std --test arch_ext
#   cargo test -p soso-llm-core --features std --test asr
#   cargo test -p soso-audio --features std
#   cargo test -p gguf2som --features std -- convierte_gguf_qwen35
# Hostrun MoE sintético:
#   cargo run -q --release -p mkmodel-soso -- --moe target/tiny-moe-model
#   cargo run --release -p soso-llm-core --features std --example hostrun -- target/tiny-moe-model @bos 4

# End-to-end (host tests ∥ build user/kernel, luego 4 shards QEMU en paralelo)
cargo xtask test
# La suite debe quedar en verde (stdin/SSH aislado por sesión desde A1).
# Ante regresiones: target/test-{llm-dense,llm-moe,sys,reclaim}-serial.log

# Pre-commit / CI equivalente:
cargo xtask check
# CI: host-and-build + qemu-sys (init) + qemu-shards (4 shards TCG) en cada PR/push. e2e-live: workflow_dispatch o cron lunes.

# QEMU: auto `-accel kvm` si /dev/kvm legible; forzar TCG para comparar:
SOSO_QEMU_ACCEL=tcg cargo xtask test

# Paralelismo QEMU (default 4 con KVM, 2 en TCG; 1 = secuencial para depurar)
SOSO_TEST_JOBS=1 cargo xtask test

# USB/xHCI: 4 escenarios en paralelo (mismo SOSO_TEST_JOBS, imágenes copiadas)
cargo xtask test-usb

# OTA recovery (host, sin QEMU):
cargo test -p soso-update-core --features std --tests

# Matriz hardware A8 (tras arranque en placa con SOSOLOG/SOSODRV):
# El agente SÍ actualiza docs/hw-matrix.json (no pide sudo). No marques
# etapas `ok` a mano ni uses --boot-ok si no llegó a sosh.
# Esta placa: GPU gb205-dgpu 10de:2f18, WiFi ax211-wifi 8086:7f70.
cargo xtask hw-matrix parse-logs --id gb205-dgpu --sosolog SOSOLOG.TXT --sosodrv SOSODRV.TXT
cargo xtask hw-matrix parse-logs --id ax211-wifi --sosolog SOSOLOG.TXT --sosodrv SOSODRV.TXT
# ./scripts/l6-a8-collect.sh --id gb205-dgpu --pci 10de:2f18   # collect + parse si hay USB
# cargo xtask hw-matrix record-boot --id gb205-dgpu            # solo si sosh; 3 seguidos = aceptación
cargo xtask hw-matrix show
# ALIVE `ok` solo con UCODE_ALIVE_NTFY. `timeout ALIVE` / GSP=fallo → fail, no ok.

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
#   llm-dense             ask residente (carga una vez, reconexión SSH sin recargar)
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

Mapa por defecto **es** (ISO español); en QEMU con teclado US usa `kbd us` en la
kernel-shell. La consola GOP decodifica UTF-8 (una celda por carácter).

Para reproducir el arranque live entero (rootfs por USB BOT, que es donde hay
contención de `HOSTS`), levanta QEMU a mano con la imagen live como
`usb-storage` sobre `qemu-xhci` y `-smp 8`; el `-drive` de `soso-bios.img` sigue
siendo el de arranque. Ver `xtask/src/test_install.rs` para el patrón de
argumentos.

## Skills layout

Skills live in `.claude/skills/`. `.cursor/skills` mirrors them — edit under `.claude/skills/`
and sync the mirror. Tras cada etapa de un `/loop`: actualizar el skill de dominio
(`soso-architecture`, `soso-gpu`, `soso-wifi`, `soso-live`), este skill si hay
tests/comandos nuevos, y `MANUAL-USUARIO.md` si el usuario ve strings o
comportamiento distinto (skill `soso-user-manual`).

## Common issues

| Symptom | Fix |
|---------|-----|
| SSH permission denied | Use `-i target/soso_test_key` or ensure `~/.ssh/id_ed25519.pub` existed before build |
| Connection refused :2222 | Wait for `sosh — escribe 'help'`; or prior QEMU still running → `pkill qemu-system-x86` |
| Teclado muerto tras la primera tecla (placa) | Algo del camino IRQ 1 toma un `lock()` o imprime; ver «Candados y contexto de interrupción» en `soso-architecture` |
| Teclas no coinciden (QWERTY vs ñ/¿) | Mapa por defecto **es**; `kbd us` en kernel-shell para teclado americano/QEMU |
| La máquina se arrastra tras usar `soso-llm`/`ask` | En askd el `ThreadPool` se suelta tras cada respuesta (`drop_pool`); si giran al 100 %, revisar `pool.rs` (deben dormir en futex entre matvecs) |
| `ask` no arranca / imprime usage de `soso-llm` | El crt0 debe pasar a `main()` solo `argv[1..]` del blob SOSA; si llega `/bin/soso-llm askd`, askd no reconoce el subcomando. Ver `user/libsoso/src/lib.rs` |
| Fecha 1970 / `soso-hf` «reloj no utilizable» | RTC CMOS: Status B bit 2 = BCD vs binario (no siglo); century en reg `0x32`. Ver `kernel/src/arch/rtc.rs`; test `cargo test -p xtask rtc_decode` |
| `voz` / `soso-voz dictar` falla | Comprobar `tiny-asr` en `/models`; `soso-voz vozd` en `:7421`; test host: `cargo test -p soso-llm-core --features std --test asr` |
| Micrófono en QEMU | `SOSO_QEMU_AUDIO=1 cargo xtask run` (Intel HDA); sin flag el shard `sys` usa `--wav` determinista |
| `ask hola` con Mixtral se queda en puntos | Sin pool de VRAM (`pool VRAM=no`) Mixtral va a CPU. No está colgado; minutos/token. `ask :modelo tiny` o esperar. Tras reflashear, un punto por capa |
| `Could not set up host forwarding rule tcp::2222` | Puerto ocupado; `pkill qemu-system-x86` y relanzar |
| SSH output desalineada | Kernel debe enviar CRLF en `ssh::tx_push` (tty cruda) |
| SSH no reconecta tras cerrar sesión | Kernel debe hacer `reset_socket` en CloseWait/TimeWait; Ctrl-C interrumpe comandos, no cierra la sesión con `ssh -tt` |
| Kernel GPF al conectar SSH | Revisar alineación de pila en `timer_isr` antes de `net::poll` |
| Redirección `> file` no crea fichero | `vfs::create_file` debe delegar a sosofs (no stub) |
| RDRAND / crypto errors | QEMU must use `-cpu max` (xtask sets this) |
| Stale disk content | `cargo xtask mkfs` then re-run |
| `cuda-proxy`: binary not found | `cargo build -p cuda-proxy --release --target-dir target` |
| L6-H: connection refused :11400 | Start cuda-proxy; llama-server must answer `/health` on :8080 |
| L6-H: no tok/s from soso | Host is `10.0.2.2` from QEMU guest; model name must match loaded GGUF |
| `soso-llm`: shapes del index no casan (`ffn_norm: falta`, etc.) | Modelo `.som` obsoleto: `cargo run -p convert-gguf -- --check <dir>`; `cargo xtask fetch-hf …` reconvierte si falla; borrar el dir y reconvertir desde caché HF |
| `GSP=fallo` / `pool VRAM=no` en GA107 | Bring-up Ampere: FWSEC-FRTS (VBIOS PROM 0x300000) + booter_load + `GSP_INIT_DONE`; `./scripts/l6-fwsec-hostcheck.sh` en host; ver **`soso-gpu`**. Reflashear o `soso-update aplicar --local` tras Ethernet vivo |
| WiFi sin ALIVE / «ALIVE degradado» | Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh`; VFIO exige `UCODE_ALIVE_NTFY` real — **`soso-wifi`** |
| `SOSOUPD.TXT` / `SOSOKRN.BIN` duplicados en ESP | Pass 1 reserva huecos; pass 2 reutiliza la misma entrada FAT (no duplica). Regenerar con `cargo xtask package-usb-live` |
| OTA kernel atascado tras corte | `SOSOKRN.MET` + backup en `SOSOKRN.BIN`; reflashear si la ESP no tiene `.MET` (live < 0.2.2) — **`soso-live`** |
| No se ve `SOSOLOG.TXT` en el USB | Está en la ESP (p1), no en p4. Agente: `udisksctl mount -b /dev/sdX1` (**soso-live**). Usuario: `cargo xtask sosolog` |
| El live se queda en bucle «no encuentra la red» | `net::poll()` resondeaba el bus entero por vuelta al no haber NIC. Ya está: `try_attach` va limitada a 1/s y las sondas cachean. Si vuelve a pasar, mira qué `println!` se repite antes de teorizar |
| La suite se cuelga (QEMU vivo, log de serie parado hace minutos) | `kill <pid>` de ese QEMU concreto; el arnés recoge y sigue. Suele ser el shard `llm-dense` |
| Un cambio en el arranque cuelga un shard minutos después | ¿Has metido un `pci::enumerate()` post-init? Reescribe los BAR de dispositivos vivos. Usa `pci::devices()` (ver `soso-architecture`) |

## Self-hosting (ruta A)

| Comando | Acción |
|---------|--------|
| `cargo run -p soso-forja-server` | Servidor host en `:8740` (sync/build remoto; `SOSO_FORJA_RELEASE=hola-std` para la demo B3) |
| `soso-forja all --host 10.0.2.2` | Guest: sync + build; ELF (`hola-std`) → `apply` a `/bin/hola-std`; release completo → `soso-update` + halt |
| `soso-forja local` | Plan de unidades en `/src/soso/forja-unit-graph.txt` |
| `cargo test -p sosofs --features std` | sosofs host-first (rename, roundtrip) |
| `docs/SELF-HOSTING.md` | Hitos 0–4 y verificación |

Tras cambiar el formato sosofs (`SOSOFS11`), regenerar imagen: `cargo xtask mkfs`.
