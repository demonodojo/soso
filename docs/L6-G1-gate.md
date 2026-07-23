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
# Spike PCI (sin VFIO, solo enum QEMU)
cargo xtask run

# VFIO passthrough
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run

# Build capa lxdde + nouveau stub
cargo xtask lx-build nouveau
```

Generado como parte del roadmap L6 (lxdde).
