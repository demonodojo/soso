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
| `nvkm_bringup_lx.c` | **puente Ola 3** | construye `nvkm_device` real + `ga102_gsp_new` |
| `nvkm_device_lx.c` | accesor `nvkm_device_subdev` | evita la tabla de chips de engine/device |
| `nvkm/subdev/gsp/{base,ga102}.c` | **nvkm real (Ola 1)** | compila+integra |
| `nvkm/core/{option,subdev,engine,memory,mm,gpuobj,firmware}.c` | **núcleo nvkm real (Ola 1 f2)** | ctors reales, no dummies |
| `nvkm/falcon/{base,fw,ga102,gm200,gp102}.c` | **falcon real (Ola 1 f2 + Ola 2)** | ctor + funcs por chip |
| `nvkm/nvfw/{fw,hs,ls,acr,flcn}.c` | **parsers firmware (Ola 2)** | `nvfw_*` |
| `nvkm/subdev/acr/{base,lsfw,tu102,ga102,ga100,gp102,gm200}.c` | **ACR real (Ola 2)** | secuencia AHESASC→ASB |
| `nvkm/subdev/{timer,mc,top,bar,instmem,fb,mmu}/{base,vmm}.c` | **subdev base real (Ola 1 f2)** | infra genérica |

**Distinción clave:** el grafo de objetos nvkm real **se construye en runtime**
(validado: `nvkm device graph OK — subdev='gsp0'` en arranque QEMU nouveau, sin
panic). Pero el "boot" GSP efectivo y el compute siguen **soft/CPU** — falta cablear
`device->pri` con reads/writes MMIO reales (`nvkm_rd32/wr32` ↔ `gsp_mmio.c`) y ejecutar
la secuencia falcon/ACR real, que requiere HW (tras G1). G3b/G4 lo completan.

**Estado (2026-07-24): 62 fuentes nvkm/lib integradas, solo 4 dummies restantes**
(`target/g3-nvkm-undefined.txt`), **todos dependientes de HW/ROM:**
`nvbios_image`/`nvbios_shadow`/`bit_entry` (lectura VBIOS por PCI ROM/MMIO) y
`nvkm_pci_msi_rearm` (necesita `struct pci_dev` real). Se resuelven integrando la capa
MMIO/PCI real (tras G1), no con más port de ficheros. `nvkm_device_subdev`/`_engine` se
proveen como accesores mínimos en `nvkm_device_lx.c` (evita la tabla de 3268 LOC de
`engine/device/base.c` que arrastraría cientos de constructores de todos los chips).
Integrado: core completo, falcon, nvfw, ACR, mmu, fb, instmem, y la **base del motor
de cómputo** (`engine/{gr,fifo,dma,falcon}` base + canales/runlist). El `engine/gr` por
chip (contexto/métodos/fw) y el compute real llegan con HW (G4, tras G1).

Primitivas ya reales en `lxdde/shim/src/shims.c`: `snprintf`/`scnprintf`/`vsnprintf`
(formateador a buffer), `alloc_page`/`page_address`/`__free_page` (página real),
`rb_*` (vía `lib/rbtree.c`), `strncasecmp`. Force-include de `<linux/types.h>` en
`compat.h` da `bool` a toda TU (p.ej. `lib/rbtree.c`); shims `export.h`/`rcupdate.h`
para código de `lib/`.

**Gotcha de build (resuelto):** `lx_build.rs` cacheaba objetos solo por mtime del
`.c`, ignorando cambios en las cabeceras del shim → objetos obsoletos. Ahora invalida
si cualquier header bajo `lxdde/shim/include/` es más nuevo (`newest_mtime`). Si dudas
del estado, `rm target/lxdde/*.o` fuerza recompilación limpia.

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
