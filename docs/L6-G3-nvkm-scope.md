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
| Staging GEM | `lx_drm_gem_create` + `gsp_fw_stage_all` |
| Poll MMIO tu102 | `gsp_mmio.c` (`0x118128`, `0x118234`) |
| ACR lx ola2 | `acr_fw.c`, `falcon_lx.c`, `acr_lx.c` |
| Fases serial | `…→fw_staged→acr_load→acr_ahesasc→acr_asb→kick→poll→booted(_soft)` |
| Inventario nvkm ola1 | `./scripts/l6-g3-nvkm-inventory.sh nvkm_ola1.list` |
| Inventario nvkm ola2 | `./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list` |
| **nvkm GSP subdev (Ola 1) compilado e integrado** | `source.list` + shims en `lxdde/shim/include/` |
| Shims lx_emul (cortan avalancha arch) | `lxdde/shim/include/{linux,asm,soc}/…` |
| Inventario símbolos undefined | `target/g3-nvkm-undefined.txt` |
| Checklist | `cargo xtask g3-check` |

**G3b siguiente:** WPR + port `subdev/acr/*` vía `nvkm_ola2.list` (sustituir lx-native).

Log objetivo G3b (hardware real, tras G1):

```
nouveau-lx: fw gb205/gsp/... (ELF)
nouveau: GSP firmware running
nouveau-lx: GSP booted
```

(sin la palabra `soft`).
