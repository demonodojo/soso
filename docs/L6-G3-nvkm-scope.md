# L6 G3b — scope del port nvkm (sin display)

Sub-fase **G3b** de [L6-native-autonomy.md](L6-native-autonomy.md): sustituir el
boot GSP **soft** por negociación real con el firmware ELF cargado en G2/G3a.

Referencia upstream: Linux 6.6.x `drivers/gpu/drm/nouveau/nvkm/` (pinneado en
`lxdde/linux/` vía `cargo xtask lx-build`).

## Qué omitimos (no necesario para compute)

| Subsistema | LOC ~ | Motivo |
|------------|-------|--------|
| `engine/disp`, DRM KMS, fbcon | ~120k | Sin pantalla en soso |
| Nouveau DRM userspace ioctls | — | Solo kernel paths |

## Qué portar (orden sugerido)

### Ola 1 — PCI + MMIO + memoria (prerrequisito GSP)

| Módulo nvkm | Shims lxdde necesarios |
|-------------|------------------------|
| `core/device.c`, `core/subdev.c` | workqueue, timer, mutex |
| `subdevs/pci.c` | `lx_pci_*` (parcial) |
| `subdevs/bar.c` | WC MMIO (hecho en `kernel/src/lxdde/pci.rs`) |
| `subdevs/mmu.c`, `subdevs/instmem.c` | DMA, kmalloc |
| `subdevs/fb.c` (mínimo) | reserva VRAM |

**Criterio ola 1 (compile trial):** las fuentes nvkm compilan e integran en
`liblxdde.a`. **HECHO (2026-07-24)** para `nvkm/subdev/gsp/{base,ga102}.c`:
compilan contra las cabeceras privadas de nouveau + shims lx_emul mínimos en
`lxdde/shim/include/` (que cortan la avalancha de cabeceras arch del kernel), y
enlazan vía dummies `trace_and_stop` autogenerados. Ver skill `soso-gpu`.

**Ola 1 fase 2 — HECHO (2026-07-24):** portado el **núcleo nvkm** con
implementaciones reales (ya no dummies): `core/{option,subdev,engine,memory,mm,`
`gpuobj,firmware}.c`, `falcon/{base,fw}.c`, y subdevs base
`subdev/{timer,mc,top,bar,instmem,fb,mmu}/{base,vmm}.c`. El kernel enlaza en
todos los modos sin regresión (E2E verde). Símbolos reales clave:
`nvkm_subdev_ctor`, `nvkm_falcon_ctor/dtor`, `nvkm_longopt`, `nvkm_firmware_load*`.
Dummies restantes (~28, ver `target/g3-nvkm-undefined.txt`): falcon por chip
(`ga102_flcn_*`,`gm200_*`,`gp102_*` → Ola 2), parsers `nvfw_*`, `core/{device,intr}.c`,
`lib/rbtree.c` (`rb_*`), y primitivas `snprintf`/`strncasecmp`/`alloc_page`.
`subdev/pci/base.c` queda fuera (requiere `struct pci_dev` real; se usa `lx_pci_*`).

**Ola 2 — HECHO (2026-07-24):** falcon por chip `falcon/{ga102,gm200,gp102}.c`,
parsers `nvfw/{fw,hs,ls,acr,flcn}.c`, y subdev **ACR** `subdev/acr/{base,lsfw,tu102,`
`ga102,ga100,gp102,gm200}.c` (secuencia `tu102_acr_init`: AHESASC→ASB real). 34
fuentes nvkm integradas; kernel enlaza en todos los modos; E2E verde. Shims nuevos:
`completion/workqueue/wait/rbtree/ctype/atomic/ktime/mm/gfp/dma-mapping` + impls
reales (`strcspn`,`kstrto*`,`kstrndup`,`strscpy`,`strncasecmp`,`memcpy_toio`,
`for_each_set_bit`). Fix de build: `lx_build.rs` ahora invalida objetos si cambian
las cabeceras del shim (antes cacheaba solo por mtime del `.c`).

**Ola 2b — HECHO (2026-07-24):** `core/intr.c` (`nvkm_intr_*` reales), `lib/rbtree.c`
(`rb_*` reales), y primitivas reales en `shims.c`: `snprintf`/`scnprintf`/`vsnprintf`
(formateador a buffer), `strncasecmp`/`strcasecmp`, `alloc_page`/`page_address`/
`__free_page` (página real). 36 fuentes integradas; kernel enlaza en todos los modos;
E2E verde. Fix build extra: force-include de `<linux/types.h>` en `compat.h` (bool en
toda TU) y shim `export.h`/`rcupdate.h` para `lib/rbtree.c`.

**Ola 2c — HECHO (2026-07-24):** `nvkm_device_subdev` (accesor mínimo en
`nvkm_device_lx.c`, sin la tabla de 3268 LOC de `engine/device/base.c`),
`core/object.c`, `falcon/ga100.c`, `acr/gp108.c`, `fb/ram.c`, `mmu/{ummu,umem,uvmm,mem}.c`,
`bios/{base,M0203}.c`. **46 fuentes nvkm/lib integradas**; kernel enlaza en todos los
modos; E2E verde. Shims nuevos: `vmap/vunmap`, flags `GFP_USER/HIGHUSER/DMA32`.

**Dummies restantes (4, `target/g3-nvkm-undefined.txt`) — TODOS dependientes de HW/ROM:**
`nvbios_image`/`nvbios_shadow`/`bit_entry` (lectura del VBIOS por PCI ROM/MMIO,
`subdev/bios/{shadow,bit}.c`) y `nvkm_pci_msi_rearm` (`subdev/pci`, necesita `struct
pci_dev` real). Se resuelven al integrar la capa MMIO/PCI real (tras G1), no con más
port de ficheros.

**Ola 3 — HECHO (2026-07-24): grafo nvkm real construyéndose en runtime.**
`nvkm_bringup_lx.c` construye un `struct nvkm_device` real (lista subdev init,
`device->pri` = BAR0) y llama `ga102_gsp_new()` → crea el subdev GSP vía
`nvkm_subdev_ctor` + `nvkm_firmware_load` (ruta `nofw`) + `nvkm_falcon_ctor`
**reales**. Validado en arranque QEMU (modo nouveau):

```
nouveau-lx: nvkm device graph OK — subdev='gsp0' falcon.func=0x… pri=0x0
```

Sin panic, sin dummy; el SO arranca hasta la shell. La construcción no toca MMIO
(por eso funciona sin HW); en placa con GPU, `nouveau_probe` pasa el BAR0 real.
Self-test en `lx_nouveau_init_module` (`nouveau_stub.c`), puente gated en
`gsp_bringup.c` (`lx_nvkm_build_gsp`).

**Ola 4 (G4) infraestructura base — HECHO (2026-07-24):** integrada la base del
motor de cómputo y del fifo/canales: `engine/{falcon,dma/{base,user}}.c`,
`engine/gr/base.c`, `engine/fifo/{base,chid,runl,runq,cgrp,chan,ucgrp,uchan}.c`,
y core object-model `core/{event,oproxy,ramht,uevent}.c`. Accesor
`nvkm_device_engine` añadido a `nvkm_device_lx.c`. **62 fuentes nvkm/lib integradas**;
solo **4 dummies restantes, TODOS HW/ROM** (`nvbios_image`/`nvbios_shadow`/`bit_entry`
= VBIOS por PCI ROM, `nvkm_pci_msi_rearm` = PCI MSI). Shims nuevos: bitmap ops
(`find_first_zero_bit`, `__set_bit`), `atomic_inc_return`.

Siguiente (requiere G1/HW): cablear `device->pri` con reads/writes reales
(`nvkm_rd32/wr32` ↔ `gsp_mmio.c`), ejecutar la secuencia falcon/ACR real (reset,
WPR, RPC) → boot GSP no-soft; luego el `engine/gr` por chip (contexto gráfico,
métodos, firmware gr) para el canal de cómputo real (G4 completo).

### Ola 2 — ACR + falcon (lx-native, antes del port nvkm completo)

| Módulo lx | Notas |
|-----------|-------|
| `acr_fw.c` | Carga `ga102/acr/ucode_ahesasc.bin` + `ucode_asb.bin` |
| `falcon_lx.c` | MMIO falcon ga102 + DMA HS v2 |
| `acr_lx.c` | Secuencia `tu102_acr_init`: AHESASC (SEC2) → ASB (GSP) |

Referencia nvkm (inventario incremental): `nvkm_ola2.list` →
`subdev/acr/{base,tu102,ga102}.c`, `falcon/{ga102,gm200,fw,base}.c`.

**Criterio ola 2:** log `acr_ahesasc` / `acr_asb` en serial; falcon mbox; WPR nvkm
sigue pendiente para boot hw real.

### Ola 3 — GSP firmware loader (port nvkm)

| Módulo nvkm | Notas |
|-------------|-------|
| `subdevs/gsp/*.c` | Handshake con blobs `bootloader`, `fmc`, `gsp` |
| `core/firmware.c` | Enlazar a `lx_request_firmware` (ya en `firmware.rs`) |

**Criterio ola 3:** log distinto de `(soft)`; registro GSP responde; dmesg-style
`nouveau: GSP firmware version …` en serial.

### Ola 4 — Canal compute (G4)

| Módulo nvkm | Notas |
|-------------|-------|
| `engine/gr/*.c` | Colas, channels, kickoff |
| `engine/falcon.c` (si aplica GB205) | ucode auxiliar |

**Criterio ola 4:** `lx_nouveau_submit_saxpy` ejecuta en GPU (no loop CPU).

## Inventario de símbolos (workflow)

```bash
cargo xtask lx-build nouveau
# Añadir .c de nvkm a lxdde/ports/nouveau/source.list de uno en uno
nm target/lxdde/liblxdde.a | rg ' U '   # undefined → stub o shim real
```

Cuando `lx_emul_trace_and_stop` salta:

1. Identificar símbolo Linux faltante.
2. Implementar shim mínimo en `kernel/src/lxdde/` o `lxdde/shim/src/shims.c`.
3. Rebuild y repetir.

## GB205 / Blackwell — riesgos

- Chipset `10de:2f18` es **reciente**; nouveau upstream puede estar detrás del
  driver propietario.
- Fallback de referencia: Ampere (`ga102`) para validar el pipeline nvkm antes de
  GB205.
- Si GSP gb205 falla en G3b: documentar registro/MMIO y comparar con trace Linux
  (`nouveau` + same firmware version 570.144).

## Estimación

| Ola | Esfuerzo |
|-----|----------|
| 1 | 4–8 semanas |
| 2 (ACR lx) | 2–4 semanas |
| 3 (GSP nvkm) | 8–16 semanas |
| 4 | 4–8 semanas |

Total G3b+G4: **6–12 meses** con hardware iterativo (coincide con PLAN L6).

## Transición desde G3a (soft)

Archivos actuales:

| Archivo | Rol hoy | Tras G3b |
|---------|---------|----------|
| `lxdde/ports/nouveau/gsp_bringup.c` | Validación ELF + soft boot | Delega en nvkm o se elimina |
| `lxdde/ports/nouveau/nouveau_stub.c` | Entry probe | Enlaza `nvkm_device_init` |
| `kernel/src/lxdde/gpu.rs` | Puente Rust↔C | Sin cambio de API |

## Estado G3 (2026-07-24)

| Componente | Ubicación |
|------------|-----------|
| Carga firmware persistente | `lxdde/ports/nouveau/gsp_fw.c` |
| Staging GEM (todo menos el ucode) | `lx_drm_gem_create` + `gsp_fw_stage_all` |
| Imagen GSP-RM + radix3 | `lxdde/ports/nouveau/gsp_rm.c` |
| Bootloader + WPR meta | `lxdde/ports/nouveau/gsp_wpr.c` |
| Ruta FMC/FSP (Blackwell) | `lxdde/ports/nouveau/fmc_lx.c` |
| Verificación en host (sin GPU) | `./scripts/l6-g3-gsp-hostcheck.sh` |
| Poll MMIO tu102 | `gsp_mmio.c` (`0x118128`, `0x118234`) |
| ACR lx ola2 (Ampere) | `acr_fw.c`, `falcon_lx.c`, `acr_lx.c` |
| Fases serial | `…→fw_staged→rm_radix3→[fmc_parse→fmc_ready→wpr_meta \| acr_*]→kick→poll→booted(_soft)` |
| Inventario nvkm ola1 | `./scripts/l6-g3-nvkm-inventory.sh nvkm_ola1.list` |
| Inventario nvkm ola2 | `./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list` |
| **nvkm GSP subdev (Ola 1) compilado e integrado** | `source.list` + shims en `lxdde/shim/include/` |
| Shims lx_emul (cortan avalancha arch) | `lxdde/shim/include/{linux,asm,soc}/…` |
| Inventario símbolos undefined | `target/g3-nvkm-undefined.txt` |
| Checklist | `cargo xtask g3-check` |

## Ruta de arranque de Blackwell (GB20x): FSP + GSP-FMC

GB20x no arranca el GSP con el ACR de SEC2 (eso es Ampere: en GB205 el falcon ni
ejecuta, `mbox0=0xbadf4100`). Lo arranca el **FSP**, al que se le manda un mensaje
**COT** por EMEM con la imagen GSP-FMC y su cadena de firma. Referencia upstream
(no está en el 6.6 pinneado): `nvkm/subdev/fsp/{gh100,gb202}.c`, `nvkm/subdev/gsp/gh100.c`.

Cadena completa y en qué punto está:

| Paso | Qué es | Dónde | Estado |
|------|--------|-------|--------|
| 1 | Validar el ELF `fmc-*.bin` (cabecera fija, 6 secciones, CRC en `sh_info`) | `fmc_lx.c` | hecho (48/97/96 = variante gb202, `cot.version=2`) |
| 2 | Leer el FSP: `NV_THERM_I2CS_SCRATCH` gb202 en **0x00ad00bc** (éxito `0xff`), colas en `0x008f2c00/04/80/84` | `fmc_lx.c` | hecho en HW: `0xff` + colas a 0 |
| 3 | **radix3** sobre `.fwimage` del ucode GSP-RM | `gsp_rm.c` | hecho, verificada en memoria |
| 4 | Bootloader RISC-V en sysmem + `GspFwWprMeta` | `gsp_wpr.c` | hecho, verificado en memoria |
| 5 | libos boot args (`GSP_FMC_BOOT_PARAMS`) | — | pendiente |
| 6 | Enviar el COT al FSP | — | pendiente (**no antes de 5**) |

Los pasos 3–5 se construyen y verifican **en memoria**; de registros, solo se
**leen** (el estado del FSP en el 2, el tamaño de VRAM en el 4). El 6 es el
primero que escribe: mandar un COT con punteros a estructuras sin construir es lo
único que hay que no hacer.

### Paso 3: radix3 (`gsp_rm.c`)

El ucode `gsp-570.144.bin` es un ELF64 (REL, RISC-V) con `.fwimage` (0x3c99000 B,
~60,5 MiB, ya múltiplo de página) y una firma por familia — `.fwsignature_gb20x`
para Blackwell, `.fwsignature_ga10x` para Ampere, 4 KiB cada una. El GSP no lee
esa imagen linealmente: recibe la física de la raíz de una tabla de 3 niveles de
`u64` cuyas hojas listan la física de **cada página** de la imagen, así que el
firmware no necesita ser físicamente contiguo. Copia exacta de
`nvkm_gsp_radix3_sg` (`nvkm/subdev/gsp/r535.c`).

Para gb205: 15513 páginas → hoja 126976 B (31 páginas), nivel 1 y raíz 4096 B cada
uno. `radix3_verify()` releé lo escrito antes de que llegue a manos del GSP
(alineación de los tres niveles, raíz→nivel 1, nivel 1→hojas entrada a entrada, y
muestreo de la primera/media/última hoja contra `lx_virt_to_phys`): un puntero mal
puesto aquí es un DMA a memoria ajena que no se vería hasta el COT.

Dos apoyos nuevos en la capa lx (`kernel/src/lxdde/mem.rs`): `lx_alloc_pages_exact`
(buffer alineado a página — `lx_kmalloc` solo alinea a 8, y la imagen tiene que
empezar en un límite de página) y `lx_virt_to_phys`. Los tres niveles sí son
contiguos y van por `lx_dma_alloc_coherent`.

**Presupuesto de heap:** el ucode ya **no** se copia a un GEM (60,6 MiB que el GSP
lee por DMA, no un objeto gráfico) y el blob en bruto se suelta
(`gsp_fw_release_one(GSP_FW_UCODE)`) en cuanto `gsp_rm_prepare` tiene su copia
alineada. Pico ≈125 MiB en vez de ≈190 MiB, con el heap en 256 MiB.

### Paso 4: WPR meta (`gsp_wpr.c`)

Referencia: `gh100_gsp_wpr_meta_init` (`nvkm/subdev/gsp/gh100.c`) y la estructura
de `nvkm/subdev/gsp/rm/r570/nvrm/gsp.h`. La versión de firmware coincide: el
`nvkm_gsp_fwif` de gh100 es literalmente `"570.144"`, la misma que los blobs de
esta máquina.

La sorpresa agradable: **en la ruta FMC el driver no calcula direcciones de WPR**.
El flag `offset_set_by_acr` de `r570_wpr_libos3_gb20x` dice que las pone el propio
FMC. Nosotros solo rellenamos punteros a sysmem y tamaños; todo lo demás
(`gspFwWprStart/End`, `gspFwOffset`, `frtsOffset`, `nonWprHeapOffset`, `fbSize`…)
se queda a cero, y `wpr_meta_verify()` **falla si algo de eso viene escrito** — un
layout inventado sería peor que ninguno. Nada que ver con la ruta Ampere de
`tu102_gsp_oneinit`, que sí construye la partición entera y arranca FWSEC-FRTS.

`struct gsp_wpr_meta` mide exactamente 256 B; lo garantizan asserts de compilación
sobre `sizeof` y sobre los offsets de `fbSize`/`pmuReservedSize`/`verified`. Los
dos `union` de upstream están puestos con su variante de arranque inicial.

Campos que sí se rellenan, y de dónde salen:

| Campo | Origen |
|-------|--------|
| `sysmemAddrOfRadix3Elf` / `sizeOfRadix3Elf` | raíz de la radix3 del paso 3 y tamaño de `.fwimage` |
| `sysmemAddrOfBootloader` / `sizeOfBootloader` | imagen RISC-V copiada a memoria coherente |
| `bootloaderCode/Data/ManifestOffset` | `RM_RISCV_UCODE_DESC` dentro del blob |
| `sysmemAddrOfSignature` / `sizeOfSignature` | firma de la familia, del paso 3 |
| `gspFwHeapSize` | `tu102_gsp_wpr_heap_size()` con la VRAM real |
| `nonWprHeapSize` = 0x220000, `pmuReservedSize` = 0x1820000 | constantes de `r570_wpr_libos3_gb20x` |
| `frtsSize` = 0x100000, `vgaWorkspaceSize` = 128 KiB | fijos en `gh100_gsp_wpr_meta_init` |

El bootloader (`bootloader-570.144.bin`) lleva la cabecera NVIDIA `nvfw_bin_hdr`
seguida de un `RM_RISCV_UCODE_DESC`. En el blob real: `bin_magic=0x10de`,
`data_offset=0x6c` + `data_size=0x31000` = el fichero exacto; `version=5` (firma la
imagen RISC-V entera como código, Hopper+), manifest 0..0xa00, datos 0xa00..0xb200,
código 0xb200..0x30a00. Los tres offsets se comprueban contra `data_size` antes de
copiar nada.

**VRAM real.** `gsp_wpr_vidmem_size()` lee `0x1183a4` (MiB), que es lo que hace
`ga102_fb_vidmem_size` de Ampere a Blackwell. Alimenta el término por GB del heap y
sustituye a la tabla heurística por SKU de `vram_for_device()`, que queda solo de
respaldo cuando no hay BAR0. Con 12288 MiB: heap = 22 + 14 + 2 + 96 = **134 MiB**.

### Verificación sin GPU

`./scripts/l6-g3-gsp-hostcheck.sh` compila `gsp_rm.c` y `gsp_wpr.c` **en el host**
con la capa lx y el MMIO simulados (`tools/gsp-hostcheck/main.c`) y los corre contra
los blobs de verdad. Comprueba las hojas de la radix3 una a una, el tamaño de la
estructura, el heap, los offsets del bootloader y que los campos del FMC siguen a
cero. El ciclo en hardware pide sudo, VFIO y ~90 s; este tarda un segundo.

**G3b siguiente:** libos boot args (paso 5) — `GSP_FMC_BOOT_PARAMS` con
`gspRmDescOffset` = la física de este meta, `bootArgsOffset` = libos, y el
`GSP_ARGUMENTS_CACHED` con las colas de mensajes. En Ampere, además, port
`subdev/acr/*` vía `nvkm_ola2.list` (sustituir lx-native).

Log objetivo G3b (hardware real, tras G1):

```
nouveau-lx: fw gb205/gsp/... (ELF)
nouveau: GSP firmware running
nouveau-lx: GSP booted
```

(sin la palabra `soft`).
