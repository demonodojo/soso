# L6 G1 — Spike GPU NVIDIA: gate go/no-go

## Objetivo

Evaluar viabilidad de cómputo NVIDIA en soso antes de invertir 6–12 meses en G3–G5.

## Hardware recomendado

- **GPU:** RTX 3060/3050 (Ampere) — GSP maduro en nouveau, firmware en linux-firmware.
- **Host:** Linux con IOMMU + VFIO (`SOSO_QEMU_GPU=vfio:BB:DD.F`).

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

## Criterios go/no-go

| Criterio | Go | No-go |
|----------|-----|-------|
| Firmware GSP redistribuible | Sí (linux-firmware) | Solo blob propietario sin licencia clara |
| NV_PMC_BOOT_0 legible desde soso | Sí | BAR0 no mapeable / IOMMU roto |
| Subconjunto nvkm acotado | < 150k LOC | Dependencia circular con DRM display |
| Saxpy SASS (G4) | Lanza en < 6 sem post-G3 | GSP no arranca en 6 sem |

## Salida no-go

- Motor 70B en CPU SMP+SIMD (L1–L4): 4–10 tok/s en servidor multichannel.
- Alternativa: GPU Intel/AMD con firmware documentado.

## Comandos soso

```bash
# 1) Activar IOMMU en GRUB (root, luego reiniciar)
sudo ./scripts/l6-g1-enable-iommu.sh

# 2) Tras reinicio: checklist (debe mostrar grupos IOMMU > 0)
cargo xtask g1-check

# 3) Prueba VFIO desde TTY (Ctrl+Alt+F3; pierdes pantalla gráfica)
sudo ./scripts/l6-g1-vfio-test.sh

# Checklist host manual
cargo xtask g1-check
cargo xtask g1-check --vfio-hint

# Spike PCI (sin VFIO, solo enum QEMU)
cargo xtask run

# VFIO passthrough manual
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run

# Build capa lxdde + nouveau stub
cargo xtask lx-build nouveau
```

## Procedimiento de cierre (esta máquina)

1. **BIOS:** VT-d / Intel Virtualization Technology → Enabled.
2. **GRUB:** `sudo ./scripts/l6-g1-enable-iommu.sh` → `sudo reboot`.
3. **Verificar:** `cargo xtask g1-check` (IOMMU > 0).
4. **TTY:** Ctrl+Alt+F3, login, `sudo ./scripts/l6-g1-vfio-test.sh`.
5. **Veredicto:** anotar abajo si sale `NV_PMC_BOOT_0` o falla BAR0/VFIO.

Tras el paso 4, aunque `NV_PMC_BOOT_0` sea **go**, el veredicto global para G3–G5 sigue siendo **no-go probable** (GB205 móvil, nouveau inmaduro). El gate cierra con evidencia, no abre el roadmap GPU.

## Resultados en esta máquina (2026-07-23)

| Item | Resultado |
|------|-----------|
| GPU | `01:00.0` **10de:2f18** — GeForce RTX 5070 Ti Mobile (GB205, Blackwell) |
| Driver host | `nvidia` (propietario) |
| IOMMU | **0 grupos** — VT-d no activo en BIOS/cmdline; bloquea VFIO |
| Firmware GSP | **34 blobs** en `/lib/firmware/nvidia/` incl. `gb205/gsp/` (symlink → ga102) |
| NV_PMC_BOOT_0 en soso | **Pendiente** — requiere IOMMU + bind VFIO + `SOSO_QEMU_GPU=vfio:01:00.0` |
| GSP en dmesg (nouveau) | Sin cargar nouveau en esta sesión (driver nvidia activo) |

**Nota hardware:** el gate recomienda Ampere (RTX 3060/3050); esta placa lleva Blackwell móvil — nouveau/GSP menos maduro que GA10x.

### Veredicto parcial

| Criterio | Estado |
|----------|--------|
| Firmware GSP redistribuible | **Go** (linux-firmware) |
| IOMMU + VFIO | **Pendiente** — ejecutar `scripts/l6-g1-enable-iommu.sh` + reinicio |
| NV_PMC_BOOT_0 legible | **Pendiente** — `scripts/l6-g1-vfio-test.sh` tras IOMMU |
| Subconjunto nvkm acotado | **Go** (estimación ~120k LOC sin display) |
| Saxpy SASS (G4) | **No-go** (no invertir; spike cerrado sin G3) |

### Veredicto global (pre-reboot)

**No-go para G3–G5 en este hardware** (RTX 5070 Ti Mobile / GB205, portátil,
IOMMU/VFIO frágil, nouveau Blackwell inmaduro). **Go** para motor 70B en CPU
(L1–L4). Completar pasos 1–4 arriba solo para documentar `NV_PMC_BOOT_0` y
cerrar el gate formalmente.

Generado como parte del roadmap L6 (lxdde).
