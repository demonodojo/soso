# L6 nativo — GPU autónoma en soso (G1→G5)

**Objetivo:** soso controla la NVIDIA RTX 5070 Ti Mobile (`10de:2f18`) **sin depender
de Linux ni CUDA en el host**. Inferencia acelerada vía lxdde + nvkm + kernels SASS.

L6-H (CUDA en host) queda como atajo opcional verificado; **este documento es el camino principal** hacia autonomía total. Ver [L6-H-cuda-hybrid.md](L6-H-cuda-hybrid.md).

## Desarrollo diario sin VFIO (GPU en el host)

Con el driver `nvidia` activo puedes avanzar sin soltar la dGPU:

| Qué | Comando | Notas |
|-----|---------|-------|
| Hostcheck G4d/G4e | `./scripts/l6-g3-gsp-hostcheck.sh` | ~1 s, sin sudo |
| Build nouveau | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask lx-build nouveau` | |
| Build soso | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build` | |
| QEMU | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask run` | Sin GPU en PCI → no GSP soft |
| Inferencia CUDA | L6-H: llama-server + cuda-proxy + `--cuda-host 10.0.2.2:11400` | **GO** 2026-07-27 |

El GO de G4e en hardware (readback VRAM vía CE) sigue requiriendo un ciclo VFIO puntual.

## Mapa de fases

```
G1  VFIO + BAR0          ──►  leer NV_PMC_BOOT_0 desde soso
G2  Firmware en sosofs    ──►  blobs GSP gb205 descomprimidos (.bin ELF)
G3  GSP vivo             ──►  nvkm negocia con el microcontrolador GSP
G4  Compute              ──►  CE en VRAM (G4e) → saxpy SASS (G4f)
G5  LLM híbrido          ──►  matvec offload en soso-llm (VRAM ~12 GiB)
```

## Estado actual (2026-07-27)

| Fase | Entregable | Estado |
|------|------------|--------|
| **G1** | IOMMU + VFIO + `NV_PMC_BOOT_0=0x…` | **GO** (2026-07-25): `0x1b5000a1`, familia Blackwell (gb20x), MSI Vector 16 HX |
| **G2** | `./scripts/l6-pack-firmware.sh` → ELF en rootfs | **Go** (gb205 + set ga102 del 3060) |
| **G3a** | Validación ELF + GEM staging + fases | **Go** |
| **G3b** | Boot GSP real (FSP/COT, radix3, WPR meta, libos) | **GO** (2026-07-25): `GSP booted (hw, GSP-FMC vía FSP)` en GB205 real |
| **G4a** | RPC recibir + `GSP_INIT_DONE` | **GO** (2026-07-25): `GSP-RM listo (RPC en marcha)` |
| **G4b** | RPC síncrono (`gsp_cmdq_call`) | **GO** — hostcheck + HW |
| **G4c** | Objetos RM (cliente → device → subdevice) | **GO** (2026-07-25): handles verificados en GB205 |
| **G4d** | VRAM + VA space externo + tablas VER3 | **Escrito + hostcheck** — falta validar en HW con el CE |
| **G4e** | Canal GPFIFO + CE (copia DMA en VRAM) | **Escrito + hostcheck** — GO HW aplazado (VFIO) |
| **G4f** | QMD + kernel SASS (`SYS_GPU_SUBMIT`) | **Bloqueado por toolchain** — `saxpy.sass.bin` vacío; GB205 = `sm_120`, requiere `ptxas` ≥ 12.8 |
| **G5** | tok/s GPU > CPU | Pendiente G4 |
| **L6-H** | `--cuda-host` → cuda-proxy → llama-server | **GO** (2026-07-27): ~35 tok/s en QEMU |

**GPUs soportadas (bring-up chip-aware):** **GB205 Blackwell** (`10de:2f18`, RTX
5070 Ti Mobile) y **Ampere GA10x** (RTX **3060**, GA106). La ruta nvkm real
(`ga102_gsp_new`) es la de Ampere, **madura en nouveau 6.6** — el **3060 es el
objetivo de validación recomendado** frente a la GB205 (Blackwell, más reciente).

Detalle del port G3–G4e: [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md).

## Tu siguiente paso (G4e GO en HW o G4f)

G4e ya está cableado (`gsp_chan`, `gsp_ce`) y cubierto por hostcheck. El **GO
real** (readback VRAM vía CE) requiere pasar la dGPU a VFIO; hasta entonces se
puede seguir en host sin desactivar la GPU del host.

```bash
# 1) Verificación sin GPU (≈1 s) — incluye canal GPFIFO + CE
./scripts/l6-g3-gsp-hostcheck.sh

# 2) Build del port nouveau
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask lx-build nouveau

# 3) Cuando puedas VFIO — ciclo en placa (prueba puntual o persist)
sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm   # si aún usa nvidia en el host
sudo ./scripts/l6-g1-vfio-test.sh                  # espera halt; busca log CE/readback
```

**Criterio GO G4e:** la GPU escribe un patrón en VRAM vía CE y la CPU lo lee
correctamente por el espacio de direcciones. Recomendado **antes** de G4f (SASS).

Para iterar sin cerrar el escritorio cada vez:

```bash
sudo ./scripts/l6-g1-vfio-persist.sh --enable   # + reboot
# … desarrollo …
sudo ./scripts/l6-g1-vfio-persist.sh --disable  # recuperar nvidia-smi en el host
```

Recuperar la GPU para Linux: ver [L6-G1-gate.md](L6-G1-gate.md) (sección VFIO
persist / restore).

## Ritual de cada ciclo de hardware

1. `./scripts/l6-g3-gsp-hostcheck.sh` antes de gastar un ciclo de GPU.
2. `./scripts/l6-kdump-setup.sh --enable` si aún no está (captura cuelgues).
3. Apagar soso con **`halt`** (dispara `gsp_fini`); no matar QEMU con la GPU viva.
4. En el log, **desconfiar de registros `0xffffffff`** (GPU fuera del bus).

Log serie esperado tras G3b (ya visto en placa):

```
lxdde-fw: cargado /lib/firmware/nvidia/gb205/gsp/… (ELF)
GSP booted (hw, GSP-FMC vía FSP)
GSP-RM listo (RPC en marcha)
objetos RM listos cli=… dev=… sub=…
```

## G2 — firmware en rootfs

Los blobs del host vienen comprimidos (`.zst`). soso **no descomprime zstd** en kernel;
el script de empaquetado lo hace en el host:

```bash
./scripts/l6-pack-firmware.sh --repack   # copia + zstd -d + verifica ELF gb205/ga102
cargo xtask build                        # embebe rootfs en la imagen
```

## G3 — dos sub-fases

| Sub-fase | Qué es | Criterio |
|----------|--------|----------|
| **G3a** | Carga + validación firmware ELF, BAR0 | Log `N blobs GSP validados (ELF)` |
| **G3b** | Cadena FSP/COT + GSP-RM vivo | Log `GSP booted` **sin** `(soft)` |

## G4 — compute nativo (sub-fases)

| Sub-fase | Qué es | Criterio |
|----------|--------|----------|
| **G4a–c** | RPC, objetos RM | `GSP_INIT_DONE`, alloc responde |
| **G4d** | VRAM + `FERMI_VASPACE_A` externo + VER3 | RM acepta directorio; traducción CPU OK |
| **G4e** | Canal + CE (`gsp_chan`, `gsp_ce`) | hostcheck OK; readback VRAM en HW (**siguiente**) |
| **G4f** | SAXPY SASS | `SYS_GPU_SUBMIT` con `on_gpu=1` real |

G4f requiere CUDA toolkit ≥ 12.8 en el host solo por `ptxas` (compila sin GPU;
funciona con la tarjeta en VFIO). Alternativa: validar en **RTX 3060** (Ampere).

## G5 — inferencia autónoma

- `soso-llm` offload híbrido: capas que quepan en ~12 GiB → GPU; resto → CPU/RAM
- Criterio: tok/s medido en placa **superior** al backend CPU del mismo modelo

## Relación con L6-H

| | L6 nativo (este doc) | L6-H |
|---|---------------------|------|
| Autonomía | **Total** | Depende del host Linux |
| CUDA | No (SASS propio) | Sí |
| Cuándo | Objetivo del proyecto | Inferencia ya (GO 2026-07-27) |
| Coexistencia | VFIO persist **off** en el host | Requiere driver NVIDIA en el host |

## Referencias

- Checklist G1: [L6-G1-gate.md](L6-G1-gate.md)
- Scope nvkm + historial HW: [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md)
- CUDA híbrido: [L6-H-cuda-hybrid.md](L6-H-cuda-hybrid.md)
- Capa lxdde: [lxdde/README.md](../lxdde/README.md)
- Skill operativa: `.claude/skills/soso-gpu/SKILL.md`
