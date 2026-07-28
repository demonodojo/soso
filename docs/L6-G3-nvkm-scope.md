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

**G4e** escrito + hostcheck (`gsp_chan`, `gsp_ce`); GO HW (readback VRAM vía CE)
pendiente de ciclo VFIO. Siguiente tras GO G4e: **G4f** (SASS). G3b y G4a–c ya
validados en GB205 (2026-07-25).

#### Las clases no se deducen, se preguntan (2026-07-28)

El port pedía `AMPERE_CHANNEL_GPFIFO_A` (0xc56f) y `AMPERE_DMA_COPY_A` (0xc6b5)
en una GB205, sobre un comentario que afirmaba que «GB205 comparte la ruta
Ampere en RM 570.144». Nadie lo había comprobado y es **falso**:
`nvkm/subdev/gsp/rm/gb20x.c` de nouveau usa para este chip
`BLACKWELL_CHANNEL_GPFIFO_B` (0xca6f), `BLACKWELL_DMA_COPY_B` (0xcab5) y
`BLACKWELL_COMPUTE_B` (0xcec0). La tercera también estaba mal aquí: teníamos
apuntada la **A** de compute (0xcdc0), que es de GB100.

En vez de cambiar tres `#define` por otros tres —que es la misma apuesta con
otros números—, el catálogo lo da la tarjeta: `gsp_rm_classes_probe` pide
`NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` (0x800292) sobre el device y
`gsp_rm_class_pick` coge la primera candidata que el chip reconozca, de más
nueva a más vieja. **La V2 y no la de siempre**: `GET_CLASSLIST` (0x800201)
devuelve la lista por un `NvP64` que apunta a memoria del llamante, y un puntero
al otro lado de un RPC no significa nada; la V2 lleva el array dentro de los
params (804 B, dentro de `RM_PARAMS_MAX`). Si la consulta falla no se aborta:
cada objeto pide su primera candidata **y lo dice en el log**, que es lo que se
hacía antes sin decirlo.

Ojo a la lectura del `0x3b` en este contexto: `INVALID_CLASS` es **0x22**, así
que el rechazo del canal que se vio en HW no era por la clase. Era el
`engineType` (GR5 en vez de COPY0) y ese arreglo aún no ha pasado por hardware.

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

| Módulo nvkm / lxdde | Notas |
|---------------------|-------|
| `engine/dma/*`, canal GPFIFO | **G4e** — copia CE en VRAM (hostcheck OK; GO HW pendiente) |
| `engine/gr/*.c` | **G4f** — SAXPY SASS |

**Criterio G4e:** readback correcto tras copia CE. **Criterio G4f:**
`lx_nouveau_submit_saxpy` con `on_gpu=1` real (no loop CPU).

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
| Colas, logs, RMARGS, boot params | `lxdde/ports/nouveau/gsp_libos.c` |
| Envío del COT (**escribe MMIO**) | `lxdde/ports/nouveau/fsp_lx.c` |
| Reservas coherentes compartidas | `lxdde/ports/nouveau/gsp_dma.c` |
| Ruta FMC/FSP (Blackwell) | `lxdde/ports/nouveau/fmc_lx.c` |
| Verificación en host (sin GPU) | `./scripts/l6-g3-gsp-hostcheck.sh` |
| Poll MMIO tu102 | `gsp_mmio.c` (`0x118128`, `0x118234`) |
| ACR lx ola2 (Ampere) | `acr_fw.c`, `falcon_lx.c`, `acr_lx.c` |
| Fases serial | `…→rm_radix3→[fmc_parse→fmc_ready→wpr_meta→libos_args→cot_ready→cot_sent→booted \| acr_*→kick→poll→booted(_soft)]` |
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
| 5 | Colas, logs, RMARGS, libos boot args y `GSP_FMC_BOOT_PARAMS` | `gsp_libos.c` + `fmc_lx_stage()` | hecho, verificado en memoria |
| 6 | Enviar el COT al FSP por EMEM y esperar al FMC | `fsp_lx.c` | **hecho y validado en HW** |

Los pasos 3–5 se construyen y verifican **en memoria**; de registros, solo se
**leen** (el estado del FSP en el 2, el tamaño de VRAM en el 4). **El 6 es el
primero que escribe en la GPU.**

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

### Paso 5: libos boot args (`gsp_libos.c`)

Referencias: `r535_gsp_libos_init`, `r535_gsp_shared_init` y `r570_gsp_set_rmargs`.
**Cuidado con la versión**: r535 y r570 no comparten el layout de
`GSP_ARGUMENTS_CACHED` ni de `MESSAGE_QUEUE_INIT_ARGUMENTS` — r570 quita los campos
`locklessCmdQueueOffset`/`locklessStatQueueOffset` y añade `bDmemStack`. Con el
firmware 570.144 va el de r570; usar el de r535 desplazaría todos los campos.

Cuatro piezas encadenadas:

1. **Memoria compartida** (`shared_init`): un único bloque contiguo con la tabla
   de PTEs delante y las dos colas de 256 KiB detrás. Salen 129 PTEs — 128 páginas
   de colas más la página de la propia tabla, que se describe a sí misma — y 63
   mensajes de 4 KiB por cola (la primera página es la cabecera). Solo se rellena
   la cabecera `msgqTxHeader` de la cola de comandos, con `rxHdrOff` apuntando al
   `readPtr` de la otra; la cola de mensajes la inicializa el GSP.
2. **Búferes de log** LOGINIT / LOGINTR / LOGRM, 64 KiB cada uno. Aunque son
   contiguos, GSP-RM espera dentro de cada búfer la lista de físicas de sus
   páginas, justo detrás del *put pointer* de la primera palabra (`create_pte_array`).
3. **RMARGS**: `GSP_ARGUMENTS_CACHED` con la física de la memoria compartida, el
   número de PTEs y los offsets de las dos colas.
4. **Boot args de libos**: una página con cuatro `LibosMemoryRegionInitArgument`
   —los tres logs y RMARGS— identificadas por su nombre empaquetado en un `u64`
   big-endian (`"LOGINIT"` → `0x4c4f47494e4954`).

Encima va el `GSP_FMC_BOOT_PARAMS`, que es lo que el COT entrega al FMC:
`bootGspRmParams` apunta al WPR meta del paso 4 (target = *coherent system*) y
`gspRmParams.bootArgsOffset` a la página de regiones (target = *noncoherent
system*). El `wprCarveout` se queda a cero — lo talla el FMC — y `libos_verify()`
falla si aparece escrito, por el mismo motivo que en el paso 4.

Los layouts de las siete estructuras de firmware están fijados con asserts de
compilación (`sizeof` de la región = 32, `msgqTxHeader` = 32, `rxHdrOff` = 32,
`bDmemStack` en 48, `profilerArgs` en 56, `gspRmParams` en 40, boot params = 80).

**Staging del FMC** (`fmc_lx_stage`): la imagen del ELF `fmc-*.bin` y las tres
partes de su cadena de firma se copian a memoria coherente, porque el FSP las lee
él mismo y no valen punteros al heap. Con esto, el paso 6 no tiene que construir
nada: solo hablar por EMEM.

Reserva total del paso 5: 516 KiB de colas + 192 KiB de logs + 8 KiB de
libos/RMARGS + 200 KiB del FMC.

### Paso 6: el COT (`fsp_lx.c`)

Referencias: `gh100_fsp_boot_gsp_fmc`, `gh100_fsp_{send,recv,poll,wait,send_sync}`
(`nvkm/subdev/fsp/gh100.c`), `gb202_fsp` para los tamaños, `gp102_flcn_emem_pio`
(`nvkm/falcon/gp102.c`) y `gh100_gsp_lockdown_released` (`subdev/gsp/gh100.c`).

El mensaje es MCTP/NVDM: dos DWORD de cabecera (`0xc0000000` = SOM|EOM;
`0x1410de7e` = tipo vendor-PCI 0x7e, vendor 0x10de, NVDM 0x14 = COT) seguidos de un
`NVDM_PAYLOAD_COT` **packed de 860 B** → 868 B en total. Los campos de firma miden
384 B aunque gb20x solo llene 48/97/96: el resto va a cero. `frtsVidmemOffset` es un
offset **desde el final de la VRAM** y vale `rsvd_size` = `ALIGN(nonWprHeap +
pmuReserved, 2 MiB)` = 0x1C00000 en gb205.

Transporte: EMEM del falcon del FSP (base `0x8f2000`), puerto/dato en `0xac0`/`0xac4`
con autoincremento (bit 24 escritura, bit 25 lectura). Se escribe el paquete,
`QUEUE_TAIL` = tamaño−4 y `QUEUE_HEAD` = 0, que es el timbre. La respuesta llega por
la cola de mensajes y se valida entera: SOM/EOM, cabecera NVDM de tipo
`FSP_RESPONSE` (0x15), `commandNvdmType` = COT y `errorCode` = 0.

Después el FMC arranca y baja el lockdown del bootrom RISC-V del GSP: se sondea
`MAILBOX0` (falcon del GSP en `0x110000`) hasta que deje de valer `0xbadf41xx` y
`HWCFG2` bit 13 (`RISCV_BR_PRIV_LOCKDOWN`) caiga.

**Guardas antes de escribir nada**: FSP con secure boot `0xff`, las cuatro colas
ociosas, payload completo y cadena de firma con los tamaños de gb20x. Todas las
esperas están acotadas (1 s para la cola y la respuesta, 4 s para el lockdown); no
hay ningún bucle que pueda quedarse girando. Cualquier fallo devuelve −1 y el
bring-up sigue a `booted (soft)` como antes.

**Caso ambiguo heredado de upstream**: `gh100_gsp_lockdown_released` acepta que
`MAILBOX0` lleve la dirección de los boot params, pero el llamante trata cualquier
`mbox0` distinto de cero como fallo. Se replica tal cual —no toca inventarse una
corrección sin poder probarla— pero el log avisa explícitamente si se da ese caso.

### Primera prueba en hardware (2026-07-25)

El FSP **aceptó el COT** a la primera: `COT aceptado por el FSP`, con la respuesta
NVDM bien formada. Eso valida de golpe el paquete, el transporte por EMEM y la
cadena de firma contra silicio real.

Lo que falló fue lo siguiente: `GSP-FMC no arrancó a tiempo (mbox0=0x00000000)`. Y
no era la GPU, era el reloj. `timer::sleep_ms` (en `kernel/src/lxdde/timer.rs`)
registraba un timer y bloqueaba la fibra actual, pero `fiber::yield_now()`
**retorna inmediatamente cuando solo hay una fibra** — el caso del bring-up. Los
4000 `lx_mdelay(1)` de la espera del lockdown se ejecutaron en unos 16 ms en vez de
en 4 s. El mismo fallo afectaba a `gsp_mmio_poll_ready`.

Arreglado: `sleep_ms` gira sobre `pit::uptime_ms()` cediendo a otras fibras cuando
las hay, y detecta una sola vez si el PIT está parado (`CLOCK_DEAD`) para no colgar
el arranque. La espera del lockdown pasa a 8 s y registra `mbox0`/`mbox1`/`HWCFG2`/
`CPUCTL` cada segundo y al agotarse.

### Segunda prueba en hardware (2026-07-25): la GPU se cae del bus

Con la espera ya de verdad, el arranque del FMC **empieza**:

```
GSP tras el COT  mbox0=0x00000000 hwcfg2=0x8187a7f7 (lockdown=1) cpuctl=0x00000180 (halted=0)
GSP esperando    (igual)
GSP-FMC falló (mbox0=0xffffffff)
```

`halted=0` dice que el core RISC-V del GSP **está corriendo**: el FSP aceptó el COT
y el FMC arrancó. Pero al cabo de ~1 s **todo el MMIO pasa a leerse `0xffffffff`** —
`mbox0`, `boot0`, `0x118128`, `0x118234`—, que es la firma de una GPU caída del bus
(reset, enlace PCIe abajo o desaparición). El `nvidia: sin GPU NVIDIA en PCI` del
resto del arranque lo confirma. La tarjeta se recupera al salir QEMU (VFIO la
resetea al cerrar el fd).

**Y produjo un GO falso.** `gsp_mmio_poll_ready` comprobaba `(a & 1) && ((b & 0xff)
== 0xff)`, condición que `0xffffffff` cumple al pie de la letra: el checklist dio
`G3b hw boot GO` con la tarjeta muerta. Corregido en tres sitios: `gsp_mmio_alive()`
trata el all-ones como silencio del bus, el bucle del lockdown aborta con un mensaje
propio en cuanto lo detecta, y `g3-check` degrada el criterio a PEND con aviso si
encuentra la huella en el log (`fuera del bus`, `118128=0xffffffff`, `boot0=0xffffffff`).

No hubo ni fallos DMAR ni AER ni eventos de enlace en el host. Lo que sí había era
**`nvidia-persistenced` en bucle de reinicio**, cargando y descargando `nvidia.ko`
unas 5 veces por segundo mientras la tarjeta estaba en VFIO — pares
`nvlink: Nvlink Core is being initialized` / `Unregistered` cada ~180 ms en `dmesg`,
que además tapaban cualquier otro mensaje. El daemon no puede funcionar con la GPU
en `vfio-pci`: no hay nada que persistir.

### Tercera prueba (2026-07-25): G3b en GO

Con `sudo systemctl mask --now nvidia-persistenced` el arranque salió a la primera:

```
COT aceptado por el FSP
GSP tras el COT  hwcfg2=0x8187a7f7 (lockdown=1)
GSP esperando    hwcfg2=0x8187a7f7 (lockdown=1)
GSP-FMC arrancado, lockdown liberado
GSP arrancado    hwcfg2=0x818787f7 (lockdown=0) mbox0=0x00000000
GSP booted (hw, GSP-FMC vía FSP, 12227 MiB VRAM)
```

El bit 13 de `HWCFG2` cae, `mbox0` sigue a cero (sin código de error) y no aparece
un solo `0xffffffff` en todo el log. **G3b cerrado.**

Que la causa de la caída anterior fuera el trasiego de `nvidia.ko` es la hipótesis
que encaja con todo lo observado, pero es **una sola observación**, no una
demostración. Mantener el daemon enmascarado mientras se itera en L6.

### Verificación sin GPU

`./scripts/l6-g3-gsp-hostcheck.sh` compila los módulos de los pasos 3–6 **en el
host** con la capa lx y **un FSP simulado detrás del MMIO**
(`tools/gsp-hostcheck/main.c`) y los corre contra los blobs de verdad. Comprueba
las hojas de la radix3 una a una, el tamaño del WPR meta, el heap, los offsets del
bootloader, las PTEs de la memoria compartida, el `id8` de las regiones libos, el
enlace de los boot params, que la imagen FMC copiada es idéntica a la del ELF, y —
lo que más importa ahora— **el paquete COT byte a byte**: 868 B, las dos cabeceras,
la firma en sus offsets, el relleno a cero y la FRTS. También comprueba que un
rechazo del FSP se detecta en vez de darse por bueno. También cubre **G4e**
(`gsp_chan`, `gsp_ce`: ALLOC canal/CE, geometría GPFIFO/USERD, encoder pushbuffer,
fini CE→canal→VMM). El ciclo en hardware pide
sudo, VFIO y ~90 s; este tarda un segundo, y desde el paso 6 es además la única
forma de cazar un paquete mal formado sin arriesgar un cuelgue de la GPU.

## Después de G3b: la pila RPC (`gsp_rpc.c`)

Con el GSP arrancado, GSP-RM habla por las dos colas del paso 5. **Este primer
tramo solo recibe**: escribe el puntero de lectura (que vive en nuestra propia
memoria) y, de MMIO, únicamente `app_version` antes de empezar. Enviar RPCs por la
cmdq es el escalón siguiente.

Referencias: `r535_gsp_msgq_{wait,peek,recv,get_entry}`, `r535_gsp_msg_recv`,
`r535_gsp_rpc_poll` (`rm/r535/rpc.c`) y `r535_gsp_init` (`rm/r535/gsp.c`).

Dos cabeceras por elemento: la del elemento de cola (`r535_gsp_msg`, 48 B de
metadatos de encolado) y la del RPC (`nvfw_gsp_rpc`, 32 B, con `length`, `function`
y `rpc_result`). Ambas con assert de compilación sobre su tamaño.

El detalle que se presta a error son **los punteros cruzados**: el `writePtr` de la
cola de mensajes vive en su propia cabecera `tx` (lo escribe el GSP), pero su
`readPtr` vive en la cabecera `rx` de la **cola de comandos** (lo escribimos
nosotros). Y las entradas empiezan **detrás** de la primera página del anillo, que
es la cabecera. Se avanza `ceil((length + 48) / 4096)` páginas módulo 63.

Antes de escuchar, `r535_gsp_init` publica `app_version` en el registro `0x080` del
falcon del GSP y exige que el núcleo RISC-V esté activo — bit 7 de
`NV_PRISCV_RISCV_CPUCTL` (`ga102_flcn_riscv_active`). En el arranque que cerró G3b
ese registro ya valía `0x180`, así que la condición se cumple.

El hito es ver llegar **`GSP_INIT_DONE`** (evento `0x1001`): prueba de una vez el
contrato entero de memoria compartida. Los eventos que lleguen antes —típicamente
`UCODE_LIBOS_PRINT`— se registran y se descartan. Fase serial: `rm_ready`.

### Primera prueba en HW de la recepción (2026-07-25)

**El transporte funciona.** GSP-RM manda cientos de RPCs bien formados —`function`
y `length` coherentes, cabeceras en su sitio—, así que los punteros cruzados, la
geometría del anillo y el formato de los elementos están bien.

Lo que llega, en orden: una avalancha de `GSP_POST_NOCAT_RECORD` (0x1020, 1244 B
cada uno), algún `GSP_LOCKDOWN_NOTICE` (0x101c) y `UCODE_LIBOS_PRINT`, y al final
`GSP_INIT_DONE` con **`rpc_result = 0x59` = `NV_ERR_OPERATING_SYSTEM`**. NOCAT es
el catálogo de crashes de RM: GSP-RM estaba fallando en bucle y registrándolo.

**Ojo con la numeración de eventos: r535 y r570 divergen a partir de 0x101c.** En
r535 ese código es `NVLINK_FAULT_UP` y `0x1020` ni existe; en r570 son
`GSP_LOCKDOWN_NOTICE` y `GSP_POST_NOCAT_RECORD`. Con el enum equivocado el
diagnóstico apunta a NVLink en una portátil que no tiene NVLink. Los nombres del
port salen de `rm/r570/nvrm/msgfn.h`.

**Causa:** `r535_gsp_oneinit` envía `GSP_SET_SYSTEM_INFO` y `SET_REGISTRY` por la
cmdq **antes de arrancar el GSP** (justo después de `libos_init`). Quedan encoladas
y GSP-RM las consume como parte de su propia inicialización. Nosotros no enviamos
ninguna de las dos, así que GSP-RM se inicializa a ciegas.

**Siguiente (2026-07-25, hecho):** el envío por la cmdq (`gsp_cmdq.c`) y
`SET_SYSTEM_INFO`/`SET_REGISTRY` antes del COT. `GSP_INIT_DONE` llega limpio;
G4a–c cerrados en GB205. Ver sección G4 más abajo.

Log objetivo G3b (hardware real, tras G1):

```
nouveau-lx: fw gb205/gsp/... (ELF)
nouveau: GSP firmware running
nouveau-lx: GSP booted
```

(sin la palabra `soft`).

## G4 — RPC, RM y compute (2026-07-25 →)

Tras G3b, el bring-up avanza por sub-fases. Estado al **2026-07-27**:

| Sub-fase | Módulos | Criterio | Estado |
|----------|---------|----------|--------|
| **G4a** | `gsp_rpc.c` (recv), `SET_SYSTEM_INFO`/`SET_REGISTRY` | `GSP_INIT_DONE`, `GSP-RM listo` | **GO** HW |
| **G4b** | `gsp_cmdq.c` (`gsp_cmdq_call`) | Round-trip síncrono | **GO** |
| **G4c** | `gsp_rm_obj.c` | Cliente/device/subdevice + static info | **GO** HW |
| **G4d** | `gsp_vram.c`, `gsp_vmm.c` | VER3 + `SET_PAGE_DIRECTORY` | Escrito + hostcheck |
| **G4e** | `gsp_chan.c`, `gsp_ce.c` | ALLOC canal/CE, PB, USERD; readback VRAM | **Escrito + hostcheck** |
| **G4f** | `engine/gr`, `saxpy.sass.bin` | `SYS_GPU_SUBMIT`, `on_gpu=1` | Bloqueado toolchain |

**GO G4e en HW** (cuando puedas VFIO): el CE ejercita traducción GPU, canal,
pushbuffer, timbre y semáforo sin necesitar SASS. Los mapeos de G4d/G4e en
`gsp_bringup.c` (`G4D_VA_BASE`, `GSP_CHAN_VA_BASE`, scratch) están listos para
readback.

**G4f:** `lxdde/ports/nouveau/saxpy.sass.bin` mide 0 bytes. GB205 = `sm_120`;
instalar CUDA ≥ 12.8 en el host solo por `ptxas` (funciona con GPU en VFIO), o
validar en RTX 3060 (Ampere).

**Apagado:** `gsp_fini.c` — `halt` / `SYS_GPU_SUBMIT` GFINI antes de soltar VFIO
(gotcha 6).

Verificación sin GPU: `./scripts/l6-g3-gsp-hostcheck.sh` (cubre G4d, G4e canal/CE
+ fini).
