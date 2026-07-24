---
name: soso-gpu
description: >-
  soso GPU nativa (NVIDIA nouveau/GSP, fase L6) — roadmap G1→G5, capa lxdde y
  port nvkm, firmware GSP, VFIO/IOMMU, shims de cabecera lx_emul y workflow de
  integración incremental. Úsala al tocar lxdde/ports/nouveau, kernel/src/lxdde,
  drivers GPU, scripts l6-*, firmware NVIDIA, o al avanzar los gates G1–G5.
---

# soso — GPU nativa NVIDIA (fase L6)

Objetivo: soso controla la **RTX 5070 Ti Mobile (`10de:2f18`, GB205 Blackwell)**
sin depender de Linux ni CUDA en el host. Inferencia acelerada vía lxdde + nvkm +
kernels SASS. Motor CPU (L1–L4) queda como fallback.

Docs fuente: `docs/L6-native-autonomy.md` (maestro), `docs/L6-G1-gate.md`,
`docs/L6-G3-nvkm-scope.md`, `docs/L6-H-cuda-hybrid.md`, y `PLAN-MODELOS-GRANDES.md`.

## Gates G1→G5

| Gate | Entregable | Criterio GO | Estado |
|------|-----------|-------------|--------|
| G1 | BAR0 bajo VFIO | log `NV_PMC_BOOT_0=0x…` | **BLOCK**: VT-d off en BIOS MSI → 0 grupos IOMMU |
| G2 | firmware gb205 en rootfs | `lxdde-fw: cargado …/gsp/…` | **Go** (blobs .zst→.bin) |
| G3a | firmware ELF + GEM staging + fases | `N blobs GSP validados` | **Go** (soft boot) |
| G3b | GSP real vía nvkm (sin display) | `GSP booted` **sin** `soft` | **En curso** (Ola 1 nvkm compila+integra) |
| G4 | saxpy SASS en VRAM | `SYS_GPU_SUBMIT` correcto en GPU | Pendiente |
| G5 | LLM híbrido (capas en ~12 GiB VRAM) | tok/s GPU > CPU | Pendiente |

**Bloqueador actual = G1**, y requiere acción física del usuario (no automatizable):
activar Intel VT-d en la BIOS MSI (Advanced → Integrated Peripherals → VT-d), luego
`sudo ./scripts/l6-g1-enable-iommu.sh` + reboot, `cargo xtask g1-check` (grupos IOMMU
> 0), y desde TTY `sudo ./scripts/l6-g1-vfio-test.sh`. Fallback sin IOMMU (solo BAR0,
PARTIAL): `l6-g1-vfio-noiommu.sh`.

## Capa lxdde

DDE estilo `lx_emul`: compila C (drivers Linux o first-party) → `liblxdde.a`,
enlazado al kernel Rust. `xtask/src/lx_build.rs`:
- Lee `lxdde/ports/<port>/source.list`; líneas `lxdde/…` = fuentes propias, otras =
  rutas del árbol Linux 6.6.32 en `lxdde/linux/` (tarball cacheado, ya extraído).
- Compila con clang freestanding (`-nostdinc`, `-include autoconf.h/compat.h/kconfig.h`),
  con include dirs privados de nouveau (`drivers/gpu/drm/nouveau/{include,include/nvkm,nvkm,.}`).
- **`generate_stubs()`**: dummy `lx_emul_trace_and_stop("sym")` para cada símbolo
  undefined que ningún objeto define y que no está en `provided_symbols()`. Parsea
  `nm` distinguiendo definidos vs undefined (líneas `U nombre` de 2 campos), así que
  NO duplica símbolos definidos en objetos hermanos. `provided_symbols()` lista los
  `lx_*` que resuelve la capa Rust/C en el link final del kernel.
- Puente Rust↔C: `kernel/src/lxdde/gpu.rs` (`lx_nouveau_*`), init en
  `kernel/src/lxdde/mod.rs`, modo `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau`.

## Port nouveau (`lxdde/ports/nouveau/`)

| Archivo | Rol | Nota |
|---------|-----|------|
| `gsp_bringup.c` | Máquina de fases GSP | **soft boot** (`booted_soft`); saxpy/matvec en CPU; VRAM hardcodeada |
| `gsp_fw.c` | Carga blobs GSP + staging GEM | valida ELF/magic |
| `gsp_mmio.c` | BAR0 rd32/wr32, poll, kick | `kick_boot` NO arranca HW real (solo traza) |
| `acr_fw.c`,`falcon_lx.c`,`acr_lx.c` | ACR ola2 lx-native (AHESASC→ASB) | best-effort/soft-fail |
| `nouveau_stub.c` | pci_driver + exports C | probe vendor 0x10de |
| `nvkm/subdev/gsp/{base,ga102}.c` | **nvkm real (Ola 1)** | compila+integra |
| `nvkm/core/{option,subdev,engine,memory,mm,gpuobj,firmware}.c` | **núcleo nvkm real (Ola 1 f2)** | ctors reales, no dummies |
| `nvkm/falcon/{base,fw}.c` | **falcon base real** | `nvkm_falcon_ctor/dtor` |
| `nvkm/subdev/{timer,mc,top,bar,instmem,fb,mmu}/{base,vmm}.c` | **subdev base real (Ola 1 f2)** | infra genérica |

**Distinción clave:** el núcleo nvkm ya está portado (compila+enlaza), pero el "boot"
y el compute siguen **simulados** (soft/CPU) — falta la ruta HW (falcon por chip,
nvfw parsers, intr/device, y la integración con MMIO real). G3b lo completa ola a ola.

**Dummies restantes (~28, `target/g3-nvkm-undefined.txt`):** falcon por chip
(`ga102_flcn_*`,`gm200_*`,`gp102_*` → Ola 2), `nvfw_*`, `core/{device,intr}.c`,
`lib/rbtree.c` (`rb_*`), `subdev/pci/base.c` (necesita `struct pci_dev` real; se usa
`lx_pci_*`), y primitivas `snprintf`/`strncasecmp`/`alloc_page` (shims declarados sin
impl → implementar antes del probe HW real).

## Port nvkm real — workflow de integración

Roadmap del port en `docs/L6-G3-nvkm-scope.md`: Ola 1 (PCI+MMIO+mem / subdev GSP),
Ola 2 (ACR+falcon, `nvkm_ola2.list`), Ola 3 (GSP loader), Ola 4 (compute `engine/gr`).

Bucle para añadir fuentes nvkm de Linux:
1. Inventario: `./scripts/l6-g3-nvkm-inventory.sh <lista>.list` (añade temporalmente a
   `source.list`, compila, vuelca undefined a `target/g3-nvkm-undefined.txt`, restaura).
2. Errores **de cabecera** → shim mínimo en `lxdde/shim/include/` (ver abajo).
3. Cuando compila, añadir la fuente a `source.list`; símbolos nvkm sin portar quedan
   como dummy `trace_and_stop` (enlaza). Portar/puentear en olas siguientes.
4. Verificar: `cargo xtask lx-build nouveau`, `cargo xtask build`, `cargo xtask g3-check`.

### Shims de cabecera (lección Ola 1)

Las cabeceras nvkm (`nvif/os.h`) arrastran ~35 headers reales del kernel; el pesado
`<linux/slab.h>` y `<linux/pci.h>` desencadenan la avalancha
`gfp→mmzone→spinlock→preempt→thread_info→processor/cpufeature` (core arch x86 con
per_cpu/current). **No se parchea ese core**: se **sustituyen** los headers pesados por
versiones mínimas estilo lx_emul en `lxdde/shim/include/`:
- `linux/slab.h` (mapea a `lx_kmalloc/kzalloc/kfree`; actúa de prelude incluyendo
  errno/err/string/spinlock/refcount/kref/bug/notifier/scatterlist + `container_of`),
  `linux/pci.h` (forward decls opacos), `mutex/spinlock/list/bitops/firmware/io/delay/`
  `log2/vmalloc/err/string/bug/refcount/kref/notifier/scatterlist`, wrappers `asm/`
  (`errno`,`ioctl`,`mmiowb`,`bitsperlong`,`unaligned`), y stubs vacíos de subsistemas no
  usados (i2c/clk/acpi/iommu/tegra/…).
- `autoconf.h`: `CONFIG_PGTABLE_LEVELS`, `X86_*_CACHE_SHIFT`, etc.
- `linux/compiler.h`/`kconfig.h`: `barrier`, `RELOC_HIDE`, `IS_ENABLED`, atributos.

Regla: los shims son supersets mínimos; deben servir a TODOS los ports sin regresión
(verificar `cargo xtask lx-build all` + build en cada `SOSO_LXDDE_MODE`).

## Firmware GSP

`./scripts/l6-pack-firmware.sh [--repack]`: copia blobs gb205/ga102 desde
`/lib/firmware/nvidia` y **descomprime `.zst`→`.bin`** (soso no tiene zstd en kernel).
Layout: `rootfs/lib/firmware/nvidia/gb205/gsp/{bootloader,fmc,gsp}-570.144.bin` (+
`ga102/acr/{ucode_ahesasc,ucode_asb}.bin`). Luego `cargo xtask build`.

## Comandos

```bash
cargo xtask lx-build nouveau      # compila el port (incl. nvkm Ola 1)
cargo xtask g1-check              # host: IOMMU/VFIO/firmware/BAR0
cargo xtask g3-check              # bring-up GSP: firmware, módulos, fases
./scripts/l6-pack-firmware.sh     # empaqueta firmware GSP
./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list   # inventario símbolos
# Passthrough (tras cerrar G1): SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run
```

## Camino alternativo L6-H (CUDA en host)

`docs/L6-H-cuda-hybrid.md`: atajo práctico, **no da autonomía**. La GPU corre en el
host Linux (llama-server `-ngl`); soso se conecta vía `cuda-proxy` TCP `:11400`.
`soso-llm run <modelo> --cuda-host 10.0.2.2:11400 --prompt "…"`. Coexiste con G1–G5.

## Riesgos

- GB205 es reciente; nouveau upstream puede ir por detrás. Fallback de validación:
  Ampere `ga102`.
- G1 depende de BIOS (usuario). El grueso del trabajo software (G3b) avanza sin HW.
