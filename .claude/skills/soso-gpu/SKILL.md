---
name: soso-gpu
description: >-
  soso GPU nativa (NVIDIA nouveau/GSP, fase L6) — roadmap G1→G5, capa lxdde y
  port nvkm, firmware GSP, VFIO/IOMMU, shims de cabecera lx_emul y workflow de
  integración incremental. Úsala al tocar lxdde/ports/nouveau, kernel/src/lxdde,
  drivers GPU, scripts l6-*, firmware NVIDIA, o al avanzar los gates G1–G5.
---

# soso — GPU nativa NVIDIA (fase L6)

Objetivo: soso controla la GPU NVIDIA sin depender de Linux ni CUDA en el host.
Inferencia acelerada vía lxdde + nvkm + kernels SASS. Motor CPU (L1–L4) = fallback.

**GPUs soportadas (bring-up chip-aware en `gsp_bringup.c`):**
- **GB205 Blackwell** (`10de:2f18`, RTX 5070 Ti Mobile) — hardware primario, pero
  Blackwell es reciente y nouveau va por detrás.
- **Ampere GA10x** (RTX **3060**, GA106) — **objetivo de validación recomendado**:
  la ruta nvkm real (`ga102_gsp_new`) es Ampere y está **madura en nouveau 6.6**.
- Familia detectada por `NV_PMC_BOOT_0` (arch `>>20`) con fallback por device-id.

Docs fuente: `docs/L6-native-autonomy.md` (maestro), `docs/L6-G1-gate.md`,
`docs/L6-G3-nvkm-scope.md`, `docs/L6-H-cuda-hybrid.md`, y `PLAN-MODELOS-GRANDES.md`.

## Gates G1→G5

| Gate | Entregable | Criterio GO | Estado |
|------|-----------|-------------|--------|
| G1 | BAR0 bajo VFIO | log `NV_PMC_BOOT_0=0x…` | **GO** (2026-07-25 01:19): `NV_PMC_BOOT_0=0x1b5000a1` desde soso, GB205 real |
| G2 | firmware gb205 en rootfs | `lxdde-fw: cargado …/gsp/…` | **Go** (blobs .zst→.bin) |
| G2 | firmware gb205 + set ga102 (3060) | `lxdde-fw: cargado …/gsp/…` | **Go** |
| G3a | firmware ELF + GEM staging + fases | `N blobs GSP validados` | **Go** (soft boot) |
| G3b | GSP real vía nvkm (sin display) | `GSP booted` **sin** `soft` | En HW real llega hasta ACR: fw + staging GEM OK, `AHESASC` falla (`mbox0=0xbadf4100`) → `booted (soft)` |
| G4 | saxpy SASS en VRAM | `SYS_GPU_SUBMIT` correcto en GPU | Infra `engine/{gr,fifo,dma}` base; bloqueado por G3b en HW |
| G5 | LLM híbrido (capas en ~12 GiB VRAM) | tok/s GPU > CPU | Pendiente G4 |

**G1 superado (2026-07-25).** Con VT-d activo en la BIOS y el bind persistente puesto
(`l6-g1-vfio-persist.sh --enable` + reboot), `sudo ./scripts/l6-g1-vfio-test.sh` da GO:
soso lee `NV_PMC_BOOT_0=0x1b5000a1` de la BAR0 real (`familia=Blackwell (gb20x)`).
El script no imprime nada durante los ~90s de `cargo xtask run` (todo va a
`target/g1-vfio-serial.log`): **parece colgado y no lo está**.

**`fw_loading` no era falta de firmware: era orden de arranque.** El primer G1 real dio
`lxdde-fw: no encontrado /lib/firmware/nvidia/gb205/gsp/…` → `GSP init falló
(status=fw_loading)` **con los blobs ya dentro de la imagen** (`sosofs: 61 inodes,
31843/32768 bloques`, 47 ficheros = todo `rootfs/`). La pista está en el orden del log:
el bring-up GSP imprimía en la línea 492 y `fs: sosofs montado` en la 502 —
`lx_request_firmware` resuelve por VFS (`kernel/src/lxdde/firmware.rs`) y aún no había
nada montado. Arreglado subiendo `fs::init()` por encima del bloque `lxdde` en
`kernel/src/main.rs` (queda tras los drivers de bloque y antes de `net::init()`;
`nvidia_compute::init` sigue después de lxdde porque consulta `gsp_ready()`).
Fallback sin IOMMU (solo BAR0, PARTIAL): `l6-g1-vfio-noiommu.sh`.

**Firmware chip-aware (2026-07-25).** `gsp_fw.c` tiene una tabla por familia:
Blackwell = `gb205/{bootloader,fmc,gsp}`, Ampere = `ga102/{bootloader,booter_load,
booter_unload,gsp}`; `gsp_fw_load_all(chip)` exige todos los blobs de su juego. Antes
cargaba los dos juegos y el ucode `gsp-570.144.bin` (60,6 MiB) se duplicaba —
en linux-firmware el de gb205 es un **symlink** al de ga102. El heap del kernel está
en 256 MiB (`kernel/src/mm/heap.rs`); con los dos juegos ni 256 ni 512 llegaban.
`lx_release_firmware` ya libera de verdad (era no-op). Desde el paso de radix3 el
ucode **no** se copia a un GEM y su blob en bruto se suelta con
`gsp_fw_release_one(GSP_FW_UCODE)` en cuanto `gsp_rm_prepare` tiene su copia: pico
≈125 MiB en vez de ≈190. La ruta Ampere está escrita pero **sin probar**: no hay
3060 en esta máquina.

**Ruta por familia en el bring-up.** `gsp_bringup.c` bifurca: Ampere → `run_acr_sec2()`
(el ACR de siempre); Blackwell → `run_fmc_blackwell()`. El ACR de `acr_fw.c` es de
Ampere (ucode `ga102` en SEC2) y en GB205 daba `falcon boot mbox0=0xbadf4100` — el
falcon ni ejecuta — porque GB20x arranca por GSP-FMC/FSP.

**GSP-FMC (`fmc_lx.c`), estado real.** Upstream de referencia (NO está en el árbol
6.6 pinneado; se consultó en git.kernel.org): `nvkm/subdev/fsp/{gh100,gb202}.c`,
`nvkm/subdev/gsp/gh100.c`, `include/nvhw/ref/{gh100,gb202}/dev_{fsp_pri,therm}.h`.
Datos que ya están verificados contra el blob de esta máquina:
- El ELF `fmc-570.144.bin` cumple la cabecera fija de upstream, 6 secciones, y sus
  CRC (`sh_info`) cuadran. Secciones: `hash`=48 B, `signature`=96, `publickey`=97,
  `image`=199240. Esos 48/97/96 son los tamaños COT de **`gb202_fsp`** (gh100 usa
  384/384) → GB205 va por la variante gb202, `cot.version = 2`.
- Registros (dentro de los 16 MiB de BAR0 mapeados): `NV_THERM_I2CS_SCRATCH` en
  **0x00ad00bc** para gb202 (¡no el 0x000200bc de gh100!), éxito = `0xff`;
  `NV_PFSP_QUEUE_HEAD/TAIL(0)` = 0x008f2c00/0x008f2c04; `MSGQ_HEAD/TAIL(0)` =
  0x008f2c80/0x008f2c84.
- **Leído en HW desde soso (2026-07-25)**: `FSP secure boot=0x000000ff (completo)` y
  las cuatro colas a 0 → el FSP terminó su arranque seguro, el canal EMEM está vivo y
  ocioso, y la offset gb202 es la buena (con la de gh100 no saldría justo `0xff`).
  El receptor del COT está listo y verificado; falta construir el payload.

Lo que **falta** para arrancar de verdad: el mensaje COT lleva
`gspFmcSysmemOffset` + `gspBootArgsSysmemOffset`, y esos boot args
(`GSP_FMC_BOOT_PARAMS`) apuntan al WPR meta (**ya construido**, ver abajo) y a los
libos boot args, que siguen sin existir. Hasta tenerlos, nada del port escribe
MMIO. Enviar un COT con punteros sin construir es lo único que hay que no hacer.

**radix3 (`gsp_rm.c`), hecho.** Primero de los tres bloques que faltaban. El ucode
`gsp-570.144.bin` es un ELF64 REL de RISC-V: `.fwimage` = 0x3c99000 B (~60,5 MiB, ya
múltiplo de página) + una firma de 4 KiB por familia (`.fwsignature_gb20x` /
`.fwsignature_ga10x`). `gsp_rm_prepare()` extrae las dos, copia la imagen a un buffer
alineado a página y construye la tabla de 3 niveles igual que `nvkm_gsp_radix3_sg`
(`nvkm/subdev/gsp/r535.c`): hojas = física de cada página de la imagen, así que el
firmware no tiene que ser contiguo. En gb205 salen 15513 páginas → hoja 126976 B,
nivel 1 y raíz 4096 B. `radix3_verify()` releé los tres niveles antes de que el GSP
los vea. Apoyos nuevos en `kernel/src/lxdde/mem.rs`: `lx_alloc_pages_exact`
(`lx_kmalloc` solo alinea a 8) y `lx_virt_to_phys`. Fase serial: `rm_radix3`, log
`radix3 verificada raíz=0x…`. **Sigue sin tocar un registro.**

**WPR meta (`gsp_wpr.c`), hecho.** Paso 4. Referencia `gh100_gsp_wpr_meta_init` +
`rm/r570/nvrm/gsp.h` (el `nvkm_gsp_fwif` de gh100 dice literalmente `"570.144"`, la
misma versión que los blobs de aquí). **En la ruta FMC el driver NO calcula
direcciones de WPR** (`offset_set_by_acr` en `r570_wpr_libos3_gb20x`): solo punteros
a sysmem y tamaños, y `wpr_meta_verify()` **falla si `gspFwWprStart`/`gspFwOffset`/
`frtsOffset`/`fbSize`… vienen escritos**. Estructura de 256 B exactos (asserts de
compilación sobre `sizeof` y offsets). El bootloader es `nvfw_bin_hdr` +
`RM_RISCV_UCODE_DESC`: en el blob real `data_offset=0x6c`+`data_size=0x31000` = el
fichero exacto, `version=5`, manifest 0..0xa00, datos 0xa00..0xb200, código
0xb200..0x30a00. **VRAM real** por `0x1183a4` (MiB, `ga102_fb_vidmem_size`, vale
hasta Blackwell) — sustituye a la heurística por SKU, que queda de respaldo; con
12288 MiB el heap sale 22+14+2+96 = **134 MiB**. Fase `wpr_meta`.

**Verificación sin GPU: `./scripts/l6-g3-gsp-hostcheck.sh`.** Compila `gsp_rm.c` y
`gsp_wpr.c` en el host con la capa lx y el MMIO simulados
(`tools/gsp-hostcheck/main.c`) contra los blobs reales: hojas de la radix3 una a
una, tamaño de la estructura, heap, offsets del bootloader, campos del FMC a cero.
Un segundo, frente a sudo + VFIO + ~90 s del ciclo en hardware. **Úsalo antes de
gastar un ciclo de GPU.**

Detalle y tabla de pasos 1–6 de la cadena FSP/COT en `docs/L6-G3-nvkm-scope.md`.

**Máquina del usuario (MSI Vector 16 HX AI, confirmado 2026-07-24):**
- **Híbrida**: iGPU Intel Arrow Lake (`00:02.0`, `i915`) pinta el panel; la dGPU
  NVIDIA GB205 (`01:00.0`, RTX 5070) es de render. **Pasar la dGPU a VFIO NO apaga la
  pantalla** (mi aviso genérico de "pierdes pantalla" NO aplica aquí). Pero eso **no**
  significa que la dGPU esté libre: Xorg la tiene abierta por render offload — ver el
  crash de unbind más abajo.
- La dGPU comparte grupo IOMMU con su **audio HDMI `01:00.1`**; VFIO exige ambas en
  `vfio-pci` ("group not viable" si no). `l6-g1-vfio-test.sh` bindea todas las
  funciones del slot `01:00.*` automáticamente.
- **No hay 3060** en esta máquina (solo GB205). El firmware `ga102/gsp/*` (3060) sí
  está en su linux-firmware. **Riesgo GB205**: nouveau 6.6 puede no bootear GSP
  Blackwell aunque G1 lea `NV_PMC_BOOT_0`; ruta madura = Ampere/3060.
- Estado preflight (2026-07-24, tras activar VT-d en BIOS): **DMAR presente, 30 grupos
  IOMMU**, dGPU en **grupo 10** junto a su audio HDMI (solo esas dos funciones → grupo
  viable). El kernel 7.0 activa `intel_iommu` **por defecto** al ver DMAR: no hace falta
  `l6-g1-enable-iommu.sh` ni tocar GRUB (`cmdline` sigue siendo `"quiet splash"`). Los
  checks ya no exigen `intel_iommu=on` si hay grupos > 0.
- **NO hacer unbind por sysfs de nvidia/nouveau.** PRIME está en `on-demand` (el panel
  va por la iGPU Intel), pero Xorg mantiene la dGPU abierta y `nvidia_drm` posee `fb0`:
  `echo … > /sys/bus/pci/drivers/nvidia/unbind` provocó
  `drm_WARN_ON(!list_empty(&fb->filp_head))` ×3 y un **GPF en
  `drm_framebuffer_cleanup+0x8f`** (puntero envenenado `dead000000000122`) que colgó la
  máquina — hubo reset (2026-07-24 20:22, kernel 7.0.0-28, nvidia 595.84).
  Vía segura, desde un TTY (Ctrl+Alt+F3):
  `sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm` → descarga
  `nvidia_uvm/nvidia_drm/nvidia_modeset/nvidia` (si algo usa la GPU, `modprobe -r` da
  EBUSY y no rompe nada) → luego `l6-g1-vfio-test.sh`, que ahora **rechaza** el unbind
  de drivers DRM. Vuelta atrás: `sudo ./scripts/l6-g1-vfio-restore.sh` (recarga la pila
  nvidia y recuerda arrancar `display-manager`).
- **Para iterar en G1 sin cerrar el escritorio cada vez**: bind persistente en el
  arranque con `sudo ./scripts/l6-g1-vfio-persist.sh --enable` + reboot. Escribe
  `/etc/modprobe.d/soso-l6-vfio.conf` (`options vfio-pci ids=10de:2f18,10de:2f80` +
  blacklist de la pila nvidia), `/etc/modules-load.d/soso-l6-vfio.conf` y
  `modprobe.blacklist=…` en GRUB (el blacklist de modprobe.d **no** frena una carga por
  nombre desde initramfs/gpu-manager; el parámetro de kernel sí). Antes de tocar nada
  verifica que hay otra GPU con driver (`00:02.0 i915`) y que PRIME no está en `nvidia`.
  Deshacer: `--disable` + reboot (restaura GRUB exacto, con backup `.bak.<fecha>`).
  Mientras esté activo **no hay CUDA ni nvidia-smi en el host**. `--status` no toca nada.

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
| `gsp_fw.c` | Carga blobs GSP + staging GEM | valida ELF/magic; el ucode NO va a GEM |
| `gsp_rm.c` | ELF64 del ucode → `.fwimage`/firma + **radix3** verificada | fase `rm_radix3`, sin MMIO |
| `gsp_wpr.c` | Bootloader RISC-V en sysmem + **`GspFwWprMeta`** | fase `wpr_meta`, solo lee VRAM |
| `fmc_lx.c` | Ruta FSP/GSP-FMC de Blackwell | valida el ELF FMC y **lee** el FSP |
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
./scripts/l6-g3-gsp-hostcheck.sh  # pasos 3 y 4 (radix3 + WPR meta) sin GPU ni sudo
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
