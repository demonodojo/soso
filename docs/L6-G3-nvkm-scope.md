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
Siguiente: `subdev/{pci,bar,mmu,instmem,fb}.c` → probe sin panic + instmem alloc.

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
