# L6 G1 — GPU NVIDIA: checklist de progreso

**Camino principal:** GPU autónoma en soso (G1→G5). Ver
[`L6-native-autonomy.md`](L6-native-autonomy.md).

## Objetivo

Verificar acceso BAR0 y firmware GSP en la GPU objetivo antes de invertir en G3–G5
(nvkm + compute). El gate es **checklist de avance**, no cierre del roadmap.

## Hardware objetivo (primario)

- **GPU:** `01:00.0` / `10de:2f18` — GeForce RTX 5070 Ti Mobile (GB205, Blackwell)
- **Host:** Linux con IOMMU + VFIO (`SOSO_QEMU_GPU=vfio:01:00.0`)
- **Referencia secundaria:** RTX 3060/3050 (Ampere) — GSP más maduro en nouveau

## Roadmap L6 (reabierto 2026-07-24)

| Fase | Objetivo | Criterio go |
|------|----------|-------------|
| G1 | BAR0 + NV_PMC_BOOT_0 bajo VFIO | Log `nvidia: … NV_PMC_BOOT_0=0x…` |
| G2 | Firmware gb205 en sosofs + `SOSO_LXDDE_MODE=nouveau` | `lxdde-fw: cargado …/gb205/gsp/…` |
| G3 | GSP boot vía nvkm (sin display) | Log `nouveau-lx: GSP booted` |
| G4 | Saxpy SASS en GPU | `SYS_GPU_SUBMIT` SAXPY correcto |
| G5 | matvec híbrido en soso-llm | tok/s GPU > CPU en mismo modelo |

L6-H (CUDA en host) es **opcional** — [`L6-H-cuda-hybrid.md`](L6-H-cuda-hybrid.md).

## Checklist G1 (host)

1. `lspci -nn | grep NVIDIA` — anotar BDF y device ID.
2. Cargar `vfio-pci`, bind de la GPU al driver VFIO.
3. `SOSO_QEMU_GPU=vfio:XX:YY.Z cargo xtask run` — log serie debe mostrar:
   - `nvidia: GPU .... NV_PMC_BOOT_0=0x........`
4. En host Linux con la misma GPU: `dmesg | grep -i gsp` tras cargar nouveau — listar blobs firmware.

## Superficie nvkm (sin display)

Estimación sobre Linux 6.6.32 (`drivers/gpu/drm/nouveau/`):

| Subsistema | LOC aprox. | Necesario para GSP |
|------------|------------|-------------------|
| nvkm/core, subdevs (pci, mmu, fb, instmem, bar, gsp) | ~80k | Sí |
| engine/gr (compute) | ~40k | Sí (G4) |
| engine/disp, DRM KMS | ~120k | No (omitir) |

## Comandos soso

```bash
# Empaquetar firmware GSP gb205 en rootfs (desde linux-firmware del host)
./scripts/l6-pack-firmware.sh

# Build capa lxdde + nouveau
cargo xtask lx-build nouveau

# Kernel con driver nouveau-lx
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build

# 1) Activar IOMMU en GRUB (root, luego reiniciar)
./scripts/l6-g1-preflight.sh
sudo ./scripts/l6-g1-enable-iommu.sh

# 2) Tras reinicio: checklist (debe mostrar grupos IOMMU > 0)
cargo xtask g1-check

# 3) Prueba VFIO desde TTY (Ctrl+Alt+F3; pierdes pantalla gráfica)
sudo ./scripts/l6-g1-vfio-test.sh

# VFIO passthrough manual
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run
```

## Procedimiento de cierre G1 (esta máquina)

**Placa:** MSI Vector 16 HX AI (`10de:2f18` en `01:00.0`).

Diagnóstico rápido:

```bash
./scripts/l6-g1-preflight.sh
```

### Paso A — BIOS (bloqueador actual: sin tabla DMAR)

1. Reiniciar → **Del** / **F2** → BIOS MSI.
2. **Advanced** → **Integrated Peripherals** → **Intel VT-d** (o *Virtualization Technology*) → **Enabled**.
3. Guardar (**F10**) y reiniciar.
4. Verificar: `test -r /sys/firmware/acpi/tables/DMAR && echo DMAR OK`

### Paso B — GRUB

```bash
sudo ./scripts/l6-g1-enable-iommu.sh
sudo reboot
cargo xtask g1-check    # IOMMU > 0, DMAR GO
```

### Paso C — VFIO + soso (desde TTY, no desde la sesión gráfica)

Ctrl+Alt+F3, login, luego:

```bash
cd ~/Trabajo/demonodojo/soso   # ajusta ruta
sudo ./scripts/l6-g1-vfio-test.sh
```

Criterio **GO:** log `nvidia: GPU 10de:2f18 NV_PMC_BOOT_0=0x........`

### Fallback (solo BAR0, no cierra G1 oficial)

Si VT-d no está disponible aún:

```bash
sudo ./scripts/l6-g1-vfio-noiommu.sh
```

Marca **PARTIAL** — valida lectura BAR0 pero sin aislamiento IOMMU.

### Registrar

Anotar abajo `NV_PMC_BOOT_0` y chipset id tras el test.

## Resultados en esta máquina

| Item | Resultado |
|------|-----------|
| GPU | `01:00.0` **10de:2f18** — GeForce RTX 5070 Ti Mobile (GB205, Blackwell) |
| Driver host | `nvidia` (propietario) |
| IOMMU | **BLOCK** — sin tabla DMAR (VT-d desactivado en BIOS MSI Vector 16 HX) |
| Firmware GSP | **34 blobs** en `/lib/firmware/nvidia/` incl. `gb205/gsp/` |
| NV_PMC_BOOT_0 en soso | Pendiente — requiere IOMMU + bind VFIO |
| GSP en soso (G3) | Pendiente — `SOSO_LXDDE_MODE=nouveau` + nvkm bring-up |

### Estado por criterio

| Criterio | Estado |
|----------|--------|
| Firmware GSP redistribuible | **Go** (linux-firmware) |
| IOMMU + VFIO | **Pendiente** — `scripts/l6-g1-enable-iommu.sh` + reinicio |
| NV_PMC_BOOT_0 legible | **Pendiente** — `scripts/l6-g1-vfio-test.sh` tras IOMMU |
| Subconjunto nvkm acotado | **Go** (estimación ~120k LOC sin display) |
| GSP boot (G3) | **En curso** — bring-up gb205 vía lxdde/nouveau |
| Saxpy SASS (G4) | **Pendiente** — tras G3 |
| matvec híbrido (G5) | **Pendiente** — tras G4 |

### Veredicto (2026-07-24)

**Roadmap L6 reabierto** con GB205 Blackwell móvil como hardware primario.
Motor CPU L1–L4 sigue como fallback. Completar G1 (IOMMU/VFIO) desbloquea
validación BAR0; G2–G5 avanzan en paralelo sobre lxdde.

Generado como parte del roadmap L6 (lxdde).
