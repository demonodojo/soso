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

**Placa:** MSI Vector 16 HX AI A2XWHG.

### Hardware confirmado (preflight 2026-07-24)

| Item | Detalle |
|------|---------|
| iGPU (panel) | **Intel Arrow Lake** `00:02.0` [8086:7d67], driver `i915` — **pinta el panel** |
| dGPU (render) | **NVIDIA GB205** `01:00.0` [10de:2f18] + audio HDMI `01:00.1` [10de:2f80], driver `nvidia` |
| Topología | **Híbrida** — la dGPU NO alimenta la pantalla |
| VT-d / DMAR | **BLOCK** — sin tabla DMAR (VT-d desactivado en BIOS) |
| IOMMU | 0 grupos; `intel_iommu=on` ausente; GRUB = `"quiet splash"` |
| Firmware host | **38 blobs GSP** en linux-firmware incl. `ga102/gsp/*` (Ampere/3060), `ad102`, `ga100`, `tu10x` |
| 3060 | **No presente** en esta máquina (solo la GB205) |

> **Híbrido ⇒ no pierdes pantalla.** Como el panel va por la iGPU Intel (`i915`),
> pasar la dGPU NVIDIA a VFIO **no apaga el display**. Basta con que ninguna app
> use la NVIDIA (`nvidia-smi`, CUDA, PRIME offload) al hacer el unbind; si falla,
> usa un TTY (Ctrl+Alt+F3).
>
> **Grupo IOMMU:** la dGPU y su audio HDMI (`01:00.0` + `01:00.1`) comparten grupo;
> VFIO exige **ambas** en `vfio-pci` ("group not viable" si no). `l6-g1-vfio-test.sh`
> ya bindea todas las funciones del slot `01:00.*` automáticamente.
>
> **Riesgo GB205:** aunque G1 lea `NV_PMC_BOOT_0`, el boot GSP completo de Blackwell
> puede fallar (nouveau 6.6 no conoce GB205). Ruta madura = Ampere/**3060**.

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

### Paso C — VFIO + soso

En esta máquina (híbrida, panel en Intel) puedes lanzarlo desde el escritorio,
cerrando antes cualquier app que use la NVIDIA. Si el unbind de `nvidia` falla,
usa un TTY (Ctrl+Alt+F3):

```bash
cd ~/Trabajo/demonodojo/soso   # ajusta ruta
sudo ./scripts/l6-g1-vfio-test.sh   # bindea 01:00.0 + 01:00.1 (audio) → vfio-pci
```

Criterio **GO:** log `nvidia: GPU 10de:2f18 NV_PMC_BOOT_0=0x........`

Con una **RTX 3060** en otro equipo (objetivo recomendado): asegúrate de tener
`linux-firmware` con `nvidia/ga102/gsp/*`, empaqueta (`./scripts/l6-pack-firmware.sh`)
y pasa su BDF: `SOSO_G1_BDF=<bdf-3060> sudo ./scripts/l6-g1-vfio-test.sh`.

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
| dGPU | `01:00.0` **10de:2f18** — RTX 5070 Ti Mobile (GB205, Blackwell) + audio HDMI `01:00.1` [10de:2f80] |
| iGPU (panel) | `00:02.0` **Intel Arrow Lake** [8086:7d67] `i915` — híbrido, la dGPU no pinta pantalla |
| Driver host dGPU | `nvidia` (propietario) |
| IOMMU | **BLOCK** — sin tabla DMAR (VT-d desactivado en BIOS MSI Vector 16 HX) |
| Firmware GSP | **38 blobs** en linux-firmware incl. `gb205/gsp/`, `ga102/gsp/` (3060), `ad102`, `ga100`, `tu10x` |
| NV_PMC_BOOT_0 en soso | Pendiente — requiere IOMMU + bind VFIO (todas las funciones del slot) |
| GSP en soso (G3) | **Grafo nvkm real construido en runtime** (62 fuentes); boot HW efectivo pendiente de G1 |
| GPU secundaria (3060) | **No presente en esta máquina**; objetivo de validación recomendado si se instala (GSP Ampere maduro) |

### Estado por criterio

| Criterio | Estado |
|----------|--------|
| Firmware GSP redistribuible | **Go** (linux-firmware; gb205 + set ga102 del 3060) |
| IOMMU + VFIO | **Pendiente** — `scripts/l6-g1-enable-iommu.sh` + reinicio (acción del usuario) |
| NV_PMC_BOOT_0 legible | **Pendiente** — `scripts/l6-g1-vfio-test.sh` tras IOMMU |
| Subconjunto nvkm acotado | **Go** — **62 fuentes nvkm/lib integradas** (Linux 6.6.32) |
| GSP boot (G3) | **Go (software)** — grafo nvkm real en runtime (`nvkm device graph OK`); solo 4 dummies HW/ROM |
| Saxpy SASS (G4) | **Pendiente** — infra `engine/{gr,fifo,dma}` base integrada; compute real tras G1 |
| matvec híbrido (G5) | **Pendiente** — tras G4 |

### Veredicto (2026-07-24)

**Roadmap L6 reabierto** con dos GPUs soportadas en software (bring-up chip-aware):
GB205 Blackwell móvil (primario) y **RTX 3060 Ampere (validación recomendada** por su
GSP maduro en nouveau). El port nvkm nativo (62 fuentes) compila, enlaza y construye
su grafo de objetos en runtime; el motor CPU L1–L4 sigue como fallback. El único
bloqueador para el boot GSP efectivo es **G1 (IOMMU/VFIO)** — acción del usuario en la
BIOS. Detalle del port: `docs/L6-G3-nvkm-scope.md`; skill `soso-gpu`.

Generado como parte del roadmap L6 (lxdde).
