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

G4e–G5 ya tienen **GO en GB205** bajo VFIO (2026-07-29). Tras cada reboot del host,
capar el enlace PCIe a Gen3 (`setpci` en root port `00:06.0`) antes del ciclo — ver
skill `soso-gpu`.

## Mapa de fases

```
G1  VFIO + BAR0          ──►  leer NV_PMC_BOOT_0 desde soso
G2  Firmware en sosofs    ──►  blobs GSP gb205 descomprimidos (.bin ELF)
G3  GSP vivo             ──►  nvkm negocia con el microcontrolador GSP
G4  Compute              ──►  CE en VRAM (G4e) → canal GR0 + saxpy SASS (G4f)
G5  LLM híbrido          ──►  matvec SASS por tandas de filas (VRAM ~12 GiB)
```

## Estado actual (2026-07-29)

| Fase | Entregable | Estado |
|------|------------|--------|
| **G1** | IOMMU + VFIO + `NV_PMC_BOOT_0=0x…` | **GO** (2026-07-25): `0x1b5000a1`, familia Blackwell (gb20x), MSI Vector 16 HX |
| **G2** | `./scripts/l6-pack-firmware.sh` → ELF en rootfs | **Go** (gb205 + set ga102 del 3060) |
| **G3a** | Validación ELF + GEM staging + fases | **Go** |
| **G3b** | Boot GSP real (FSP/COT, radix3, WPR meta, libos) | **GO** (2026-07-25): `GSP booted (hw, GSP-FMC vía FSP)` en GB205 real |
| **G4a** | RPC recibir + `GSP_INIT_DONE` | **GO** (2026-07-25): `GSP-RM listo (RPC en marcha)` |
| **G4b** | RPC síncrono (`gsp_cmdq_call`) | **GO** — hostcheck + HW |
| **G4c** | Objetos RM (cliente → device → subdevice) | **GO** (2026-07-25): handles verificados en GB205 |
| **G4d** | VRAM + VA space externo + tablas VER3 | **GO** HW (2026-07-29) — ejercitado por CE/compute |
| **G4e** | Canal GPFIFO + CE (copia DMA en VRAM) | **GO** (2026-07-29): `CE readback verificado (G4e GO)` |
| **G4f** | QMD + kernel SASS (`SYS_GPU_SUBMIT`) | **GO** (2026-07-29): PCAS 24 B, clase `0xcec0`, saxpy/matvec en silicio |
| **G5** | matvec SASS en `soso-llm` | **GO funcional** (2026-07-29): `tiny --max 4` → 97 matvec GPU OK; tok/s vs CPU en modelos grandes pendiente |
| **L6-H** | `--cuda-host` → cuda-proxy → llama-server | **GO** (2026-07-27): ~35 tok/s en QEMU |

**GPUs soportadas (bring-up chip-aware):** **GB205 Blackwell** (`10de:2f18`, RTX
5070 Ti Mobile) y **Ampere GA10x** (RTX **3060**, GA106). La ruta nvkm real
(`ga102_gsp_new`) es la de Ampere, **madura en nouveau 6.6** — el **3060 es el
objetivo de validación recomendado** frente a la GB205 (Blackwell, más reciente).

Detalle del port G3–G4e: [L6-G3-nvkm-scope.md](L6-G3-nvkm-scope.md).

## El toolchain de SASS ya no bloquea nada

La nota de "bloqueado por toolchain" de G4f era cierta un rato y dejó de serlo:
`ptxas` compila **sin GPU** y basta CUDA ≥ 12.8 para `sm_120`. Si no hay `nvcc` en
el PATH, `scripts/l6-g4f-build-sass.sh` usa Docker
(`nvidia/cuda:12.8.0-devel-ubuntu24.04`) y sale igual. Reproducible: el blob de
saxpy vuelve a salir byte a byte idéntico.

```bash
./scripts/l6-g4f-build-sass.sh      # saxpy.sass.bin + matvec.sass.bin y sus embeds
```

Los metadatos (`regcount`, `param_base`, offsets de los parámetros) los genera el
script **leyéndolos del cubin**; el port los compara con lo que su header declara
(`param_count`) y falla si no cuadran, porque un `.cu` con un parámetro más y un
header sin tocar es un lanzamiento que lee basura sin dar ningún error.

## Ejercitar el camino de la GPU sin GPU

`soso-llm --gpu-soft` (o `SYS_GPU_SUBMIT` con `SOFTG`) enciende un dispositivo de
cómputo software en el kernel: calcula en la CPU y devuelve `COMPUTED=1, ON_GPU=0`,
así que el offload recorre las mismas syscalls que recorrería con la tarjeta.

No prueba nada de la GPU; prueba **todo lo que la rodea**, que es justo lo que no
conviene depurar a golpe de ciclo VFIO. Al encenderlo por primera vez (2026-07-28)
salieron cinco fallos latentes, uno de ellos ajeno a la GPU: `user_range_ok` no
materializaba las páginas de un `mmap` sin estrenar, así que **cualquier** syscall
que escribiese en un búfer así contestaba EFAULT. Ver PLAN-MODELOS-GRANDES.md.

```bash
cargo xtask test          # incluye `init test` en el guest y una inferencia --gpu-soft
```

## Tu siguiente paso (GO en HW de G4e/G4f/G5)

G4e/G4f/G5 están cableados (`gsp_chan`, `gsp_ce`, `gsp_compute`) y cubiertos por
hostcheck. El **GO real** (readback VRAM vía CE, semáforo del QMD) requiere pasar
la dGPU a VFIO; hasta entonces se puede seguir en host sin desactivar la GPU del
host.

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

**Dos canales, no uno (2026-07-28).** El objeto de compute NO se puede colgar del
canal del CE: RM contesta `INVALID_CLASS` (0x22) a `BLACKWELL_COMPUTE_B` sobre un
canal atado a `NV2080_ENGINE_TYPE_COPY0` (=9) aunque la clase sea la que dice
`rm/gb20x.c` y el mismo canal acepte `BLACKWELL_DMA_COPY_B`. El compute necesita
su propio canal sobre **GR0** (=1), y por eso el bring-up levanta dos: el de COPY0
mueve el SASS a VRAM y el de GR0 ejecuta. En el log se ven los dos, cada uno con
su motor y su ventana de VAs (+0x10000).

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
| **G4e** | Canal COPY0 + CE (`gsp_chan`, `gsp_ce`) | hostcheck OK; readback VRAM en HW (**siguiente**) |
| **G4f** | Canal GR0 + SAXPY SASS | `SYS_GPU_SUBMIT` con `on_gpu=1` real |

El SASS lo compila `scripts/l6-g4f-build-sass.sh` (nvcc del host o Docker, sin
GPU). Alternativa para el GO: validar en **RTX 3060** (Ampere).

## G5 — inferencia autónoma

- `soso-llm` offload híbrido: capas que quepan en ~12 GiB → GPU; resto → CPU/RAM
- Criterio: tok/s medido en placa **superior** al backend CPU del mismo modelo

**Cómo se hace el matvec (escrito 2026-07-28).** `matvec.cu` → un hilo por fila,
`gsp_compute_matvec_f32` lo lanza por **tandas de filas**: el staging de sysmem son
512 KiB (320 KiB de matriz + 128 KiB de `x` + 4 KiB de `y`), así que con `cols`
columnas caben `320 KiB / (cols·4)` filas por tanda, con tope de 1024 por el
tamaño de `y`. Cada tanda es un QMD con su semáforo, y el pushbuffer se **rebobina**
entre tandas (`gsp_chan_pb_rewind`, sólo cuando `GPGet == GPPut`): en 4 KiB de
pushbuffer no caben más de cuatro QMD inline.

Las **filas** se trocean; las **columnas no**, porque el kernel escribe `y[r]` de
una fila entera y no acumula. Por eso el tamaño de `x` (128 KiB → **32768
columnas**) es un límite duro: cubre el FFN de un 70B (28672), y por encima de eso
`rows_per_tile` sale 0 y el matvec se hace en CPU. Con anchuras grandes la tanda es
de pocas filas (28672 columnas → 2 filas), así que la espera del semáforo sondea
20000 veces a pelo antes de dormir: con miles de tandas, un `mdelay(1)` por tanda
sería tiempo de dormir, no de calcular.

Una tanda que no señaliza **aborta el matvec entero** y `lx_nouveau_submit_matvec_f32`
lo recalcula en CPU de principio a fin. Devolver `on_gpu=1` con la mitad del vector
hecha sería un modelo escupiendo texto plausible pero mal — el único fallo de esta
ruta que no se ve venir.

Lo que el hostcheck cubre sin GPU: los dos blobs son kernels distintos, el QMD de
cada uno con SU regcount y SU programa, los cinco parámetros de matvec (incluido
que `rows` de 4 B no pise `cols`, que va pegado en +28), la aritmética de las
tandas contra las dos regiones, y un `stage`/`read` de 40 filas en tandas de 7
simulando a mano lo que escribiría la GPU — que es lo que caza un `row0` mal
puesto.

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
