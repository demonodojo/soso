# L6 G1 — GPU NVIDIA: checklist de progreso

**Camino principal:** GPU autónoma en soso (G1→G5). Ver
[`L6-native-autonomy.md`](L6-native-autonomy.md).

## Objetivo

Verificar acceso BAR0 y firmware GSP en la GPU objetivo antes de invertir en G3–G5
(nvkm + compute). G1 **cerrado** en esta máquina (2026-07-25); el gate sigue siendo
checklist de avance para otras placas o reinstalaciones.

## Hardware objetivo (primario)

- **GPU:** `01:00.0` / `10de:2f18` — GeForce RTX 5070 Ti Mobile (GB205, Blackwell)
- **Audio HDMI:** `01:00.1` / `10de:2f80` — mismo grupo IOMMU que la dGPU
- **Host:** Linux con IOMMU + VFIO (`SOSO_QEMU_GPU=vfio:01:00.0`)
- **Referencia secundaria:** RTX 3060/3050 (Ampere) — GSP más maduro en nouveau

## Roadmap L6

| Fase | Objetivo | Criterio go | Estado (MSI Vector 16 HX) |
|------|----------|-------------|---------------------------|
| G1 | BAR0 + NV_PMC_BOOT_0 bajo VFIO | Log `nvidia: … NV_PMC_BOOT_0=0x…` | **GO** — `0x1b5000a1` |
| G2 | Firmware gb205 en sosofs | `lxdde-fw: cargado …/gb205/gsp/…` | **Go** |
| G3 | GSP boot vía nvkm (sin display) | `GSP booted` sin `(soft)` | **GO** (2026-07-25) |
| G4e | CE copia en VRAM | Readback correcto desde GPU | **Pendiente** |
| G4f | Saxpy SASS en GPU | `SYS_GPU_SUBMIT` SAXPY correcto | Pendiente G4e + toolchain |
| G5 | matvec híbrido en soso-llm | tok/s GPU > CPU | Pendiente G4 |

L6-H (CUDA en host) es **opcional** — [`L6-H-cuda-hybrid.md`](L6-H-cuda-hybrid.md).

## Checklist G1 (host)

1. `lspci -nn | grep NVIDIA` — anotar BDF y device ID.
2. Cargar `vfio-pci`, bind de la GPU (y audio HDMI del mismo slot) al driver VFIO.
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

# Checklist host (IOMMU, firmware, BAR0)
cargo xtask g1-check

# Prueba VFIO (TTY si hace falta liberar la dGPU del driver propietario)
sudo ./scripts/l6-g1-vfio-test.sh

# Bind persistente para iterar sin cerrar sesión cada vez
sudo ./scripts/l6-g1-vfio-persist.sh --enable   # + reboot
sudo ./scripts/l6-g1-vfio-persist.sh --disable  # recuperar NVIDIA en Linux + reboot
sudo ./scripts/l6-g1-vfio-persist.sh --status

# VFIO passthrough manual
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run
```

## Procedimiento en esta máquina

**Placa:** MSI Vector 16 HX AI A2XWHG.

### Hardware confirmado

| Item | Detalle |
|------|---------|
| iGPU (panel) | **Intel Arrow Lake** `00:02.0` [8086:7d67], driver `i915` — **pinta el panel** |
| dGPU (render) | **NVIDIA GB205** `01:00.0` [10de:2f18] + audio HDMI `01:00.1` [10de:2f80] |
| Topología | **Híbrida** — la dGPU NO alimenta la pantalla |
| VT-d / DMAR | **Go** — VT-d activo en BIOS; **30 grupos IOMMU** (kernel 7.0 activa `intel_iommu` por defecto al ver DMAR) |
| Firmware host | Blobs GSP gb205 + ga102 (3060) en linux-firmware |
| 3060 | **No presente** en esta máquina (solo la GB205) |

> **Híbrido ⇒ no pierdes pantalla.** Como el panel va por la iGPU Intel (`i915`),
> pasar la dGPU NVIDIA a VFIO **no apaga el display**. Pero Xorg mantiene la dGPU
> abierta (PRIME render offload); **no** hagas unbind por sysfs de `nvidia` con el
> driver vivo — provoca GPF en `drm_framebuffer_cleanup`. Vía segura:
> `sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm` desde un TTY.
>
> **Grupo IOMMU:** la dGPU y su audio HDMI comparten grupo; VFIO exige **ambas**
> en `vfio-pci`. `l6-g1-vfio-test.sh` bindea todas las funciones del slot `01:00.*`.
>
> **Apagado ordenado:** nunca mates QEMU con el GSP vivo; usa `halt` en soso
> (dispara `gsp_fini`). Ver gotcha 6 en `docs/L6-G3-nvkm-scope.md`.

### Liberar / restaurar la dGPU en Linux

| Situación | Comando |
|-----------|---------|
| Prueba puntual VFIO, volver a NVIDIA sin reiniciar | `sudo ./scripts/l6-g1-vfio-restore.sh` |
| Bind persistente activo, recuperar host | `sudo ./scripts/l6-g1-vfio-persist.sh --disable` + **reboot** |
| Antes de bind VFIO en caliente | `sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm` (TTY) |

### Registrar resultado G1

| Item | Resultado |
|------|-----------|
| dGPU | `01:00.0` **10de:2f18** — RTX 5070 Ti Mobile (GB205) |
| NV_PMC_BOOT_0 en soso | **`0x1b5000a1`** (familia Blackwell gb20x) |
| IOMMU | **Go** — 30 grupos |
| GSP boot (G3b) | **GO** — `GSP booted (hw, GSP-FMC vía FSP)` |
| G4a–c (RPC + RM) | **GO** en GB205 (2026-07-25) |
| Siguiente | **G4e** — canal CE + readback VRAM |

### Fallback (solo BAR0, no cierra G1 oficial)

Si VT-d no está disponible:

```bash
sudo ./scripts/l6-g1-vfio-noiommu.sh
```

Marca **PARTIAL** — valida lectura BAR0 pero sin aislamiento IOMMU.

## Veredicto (2026-07-27)

**G1 cerrado** en MSI Vector 16 HX con GB205. El port nvkm nativo (62 fuentes)
compila, enlaza y ha completado G3b y G4a–c en hardware real. El trabajo activo
es **G4e** (CE en VRAM), luego G4f (SASS) y G5 (LLM híbrido). Detalle:
`docs/L6-G3-nvkm-scope.md`; skill `soso-gpu`.

Generado como parte del roadmap L6 (lxdde).
