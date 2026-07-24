# L6 nativo — GPU autónoma en soso (G1→G5)

**Objetivo:** soso controla la NVIDIA RTX 5070 Ti Mobile (`10de:2f18`) **sin depender
de Linux ni CUDA en el host**. Inferencia acelerada vía lxdde + nvkm + kernels SASS.

L6-H (CUDA en host) queda como atajo opcional; **este documento es el camino principal.**

## Mapa de fases

```
G1  VFIO + BAR0          ──►  leer NV_PMC_BOOT_0 desde soso
G2  Firmware en sosofs    ──►  blobs GSP gb205 descomprimidos (.bin ELF)
G3  GSP vivo             ──►  nvkm negocia con el microcontrolador GSP
G4  Compute              ──►  saxpy SASS real en VRAM (SYS_GPU_SUBMIT)
G5  LLM híbrido          ──►  matvec offload en soso-llm (VRAM ~12 GiB)
```

## Estado actual (2026-07-24)

| Fase | Entregable | Estado |
|------|------------|--------|
| G1 | IOMMU + VFIO + log `NV_PMC_BOOT_0=0x…` | **BLOCK** — 0 grupos IOMMU en placa (VT-d off en BIOS; acción del usuario) |
| G2 | `./scripts/l6-pack-firmware.sh` → ELF en rootfs | **Go** (gb205 + set ga102 del 3060, con `zstd`) |
| G3a | Validación ELF + GEM staging + fases | **Go** (soft boot si poll falla) |
| G3b | Port nvkm real | **Go (software)** — **62 fuentes nvkm/lib** integradas (core, falcon, nvfw, ACR, mmu, fb, instmem, engine gr/fifo/dma base). El **grafo de objetos nvkm real se construye en runtime** (`nvkm device graph OK — subdev='gsp0'`). Solo 4 dummies, todos HW/ROM. Falta el boot GSP efectivo (MMIO real ↔ `gsp_mmio.c`) → requiere G1 |
| G4 | Saxpy SASS en GPU | Infra base integrada (`engine/{gr,fifo,dma}`); el gr por chip + compute real requieren GSP arrancado (G1) |
| G5 | tok/s GPU > CPU | Pendiente G4 |

**GPUs soportadas (bring-up chip-aware):** **GB205 Blackwell** (`10de:2f18`, RTX
5070 Ti Mobile) y **Ampere GA10x** (RTX **3060**, GA106). La ruta nvkm real
(`ga102_gsp_new`) es la de Ampere, **madura en nouveau 6.6** — el **3060 es el
objetivo de validación recomendado** frente a la GB205 (Blackwell, más reciente).

## Tu siguiente paso (G1 en placa)

G1 es el **desbloqueador**: sin IOMMU no hay passthrough VFIO y no se valida BAR0 en hardware real.

```bash
# 1) BIOS: VT-d / Intel Virtualization Technology → Enabled
sudo ./scripts/l6-g1-enable-iommu.sh
sudo reboot

# 2) Tras reinicio (debe mostrar grupos IOMMU > 0)
cargo xtask g1-check

# 3) Desde TTY (Ctrl+Alt+F3) — pierdes la sesión gráfica
sudo ./scripts/l6-g1-vfio-test.sh

# 4) Si GO: continuar G2+G3 en QEMU con la GPU passthrough
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run
```

**Con una RTX 3060 (recomendado para validar):** instala `linux-firmware` con
`nvidia/ga102/gsp/*`, empaqueta (`./scripts/l6-pack-firmware.sh` avisa si falta el
set), y pasa la 3060 por VFIO. `nouveau_probe` reconoce vendor `0x10de`, detecta la
familia Ampere y ejercita el nvkm portado (GSP maduro) contra BAR0 real.

Log serie esperado en G1:

```
nvidia: GPU 10de:2f18 NV_PMC_BOOT_0=0x........
lxdde-fw: cargado /lib/firmware/nvidia/gb205/gsp/... (ELF)
nouveau-lx: GSP booted (soft, gb205, 12288 MiB VRAM)
```

## G2 — firmware en rootfs

Los blobs del host vienen comprimidos (`.zst`). soso **no descomprime zstd** en kernel;
el script de empaquetado lo hace en el host:

```bash
./scripts/l6-pack-firmware.sh --repack   # copia + zstd -d + verifica 3× ELF gb205
cargo xtask build                        # embebe rootfs en la imagen
```

## G3 — dos sub-fases

| Sub-fase | Qué es | Criterio |
|----------|--------|----------|
| **G3a** (hecho parcialmente) | Carga + validación firmware ELF, BAR0, estado `fw_ready` | Log `nouveau-lx: N blobs GSP validados (ELF)` |
| **G3b** (trabajo grande) | Port subconjunto nvkm (~120k LOC sin display) | Log `GSP booted` **sin** `(soft)` + respuesta GSP en MMIO |

Detalle del port: [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md).

## G4 — compute nativo

- Canal de comandos GPU → cola nvkm `engine/gr`
- Kernel saxpy en SASS Blackwell (`lxdde/ports/nouveau/saxpy.sass.bin`)
- Verificación: `SYS_GPU_SUBMIT` SAXPY con resultado correcto **en VRAM**, no CPU

## G5 — inferencia autónoma

- `soso-llm` offload híbrido: capas que quepan en ~12 GiB → GPU; resto → CPU/RAM
- Criterio: tok/s medido en placa **superior** al backend CPU del mismo modelo

## Relación con L6-H

| | L6 nativo (este doc) | L6-H |
|---|---------------------|------|
| Autonomía | **Total** | Depende del host Linux |
| CUDA | No (SASS propio) | Sí |
| Cuándo | Objetivo del proyecto | Atajo opcional |

## Referencias

- Checklist G1: [L6-G1-gate.md](L6-G1-gate.md)
- Scope nvkm: [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md)
- Capa lxdde: [lxdde/README.md](../lxdde/README.md)
