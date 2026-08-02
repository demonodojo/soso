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
| G4e | CE copia en VRAM | Readback correcto desde GPU | **GO** (2026-07-29) |
| G4f | Saxpy/matvec SASS en GPU | `SYS_GPU_SUBMIT` / PCAS con `on_gpu=1` | **GO** (2026-07-29) |
| G5 | matvec híbrido en soso-llm | matvec en GPU (tiny OK); tok/s vs CPU | **GO funcional** (2026-07-29) |

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
| G4d–G5 (CE + SASS + soso-llm) | **GO** en GB205 (2026-07-29) — ver `soso-gpu` |
| PCIe Gen4/5 post-GSP | **GO** (2026-07-30) — cap Gen3 en FMC, bump `SOSO_G1_PCIE_BUMP=4\|5` |
| G6 (pesos en VRAM) | **GO** (2026-08-02) — 192 matvec en GPU, 24 matrices residentes, 0 caídas a CPU. Lo bloqueaba un solapamiento de ventanas de VA: los pesos se mapeaban sobre el contexto de GR |
| Rendimiento G6 | Lanzamiento de QMD **10 ms → ~0,4 ms** y matvec **137 → 31,5 ms/capa** (2026-08-02) al dar al kernel un reloj fino: el tick va a 100 Hz y las esperas dormían un tick entero |
| Streaming de shards | **85 360 → 2 970 ms** en 20 tokens (0,23 → 6,73 tok/s) el 2026-08-02, midiendo en QEMU con el modelo `tiny`. Ver abajo |
| Siguiente | El matvec vuelve a mandar: 2,16 s de los 2,97 (73%). Lo que queda del streaming son 620 ms de carga en frío, y son coste único, no por token |

### Los 97% que no eran del matvec (2026-08-02)

El tiempo de inferencia no estaba en el cálculo ni en la GPU sino en la ruta de
pesos, y no salía en ningún cronómetro: `observe_layer` mide `forward_layer`,
pero `prefetch_shards` corre **antes** de arrancar el reloj y
`release_shards_except` **después** de pararlo. 160 ms de capas medidas frente a
4,3 s de token real. Lo primero fue cronometrar esas dos llamadas
(`stats.stream_ms` / `stats.release_ms`, ahora en la línea `streaming —`).

Con eso a la vista salieron cuatro averías encadenadas, todas de la misma
familia —una constante fija donde hacía falta una proporción—:

| # | Avería | Efecto |
|---|--------|--------|
| 1 | `recompute_streaming_budgets` recortaba el working-set a `DEFAULT_RESIDENT_LAYERS` (2). Ese 2 es el valor de reserva para cuando no se sabe cuánto pesa una capa, no un techo | Un modelo de 2,3 MiB con 1 GiB de presupuesto —cabe 400 veces— liberaba y remapeaba medio modelo por token. **85,4 → 46,5 s** |
| 2 | `keep_shards_after` extendía la ventana sólo hacia atrás (`end = layer + 1`), así que en la capa 0 liberaba las capas 2 y 3 recién mapeadas | Dos desalojos y sus refaltos por token aunque el working-set fuera de 4 capas. **46,5 → 7,2 s** |
| 3 | `BlockCache` daba a los shards (`CACHE_STREAM`) una ventana fija de 32 bloques —128 KiB— y `last_prefetch` era una sola casilla, que no sobrevive a un acceso que alterna entre shards | Cada prefetch leía el shard siguiente entero y sólo podía quedarse con 32 bloques. **7,2 → 5,4 s** |
| 4 | El constructor de sosomfs ponía `prefetch_bytes = 8 MiB` fijo para todos los shards, cuando el siguiente ocupa decenas de KiB | Cada prefetch arrastraba 2048 bloques de disco y desalojaba lo útil. **5,4 → 3,0 s** |

Medido a nivel de dispositivo, la amplificación de lectura era de **94 bloques
leídos por cada bloque de modelo**: 54 272 lecturas de 4 KiB a ~177 us —9,2 s de
disco— para un modelo de 577 bloques. Tras (3) y (4) quedan ~1 100.

Lección de método: el corte se encontró bajando un nivel cada vez y **midiendo**
—faltas de página por ventana, no acumuladas (el acumulado bajaba de 13 a 4 ms y
escondía que las 512 primeras costaban 12 ms y el resto 0,8); luego el coste
dentro del FS; luego las lecturas reales al dispositivo—. Las tres primeras
hipótesis por inspección del código (CRC de segmento, lecturas sector a sector,
walk de metadatos) eran plausibles y las tres estaban equivocadas.

### Qué mirar en el próximo ciclo de VFIO (2026-08-01)

El ciclo del 30-jul se fue sin diagnóstico porque **nadie drenaba el anillo de
mensajes**: RM cuenta los fallos de canal por evento y el `RC_TRIGGERED` con su
`mmuFault` se quedó en la cola. Eso ya está arreglado; en el log hay que buscar,
por este orden:

1. `SONDA GPU (sysmem)` y `SONDA GPU (VRAM)` — dos matvec iguales, uno con los
   pesos en sysmem y otro residentes. Si el primero va y el segundo no, el
   problema es la ventana G6 y no el compute.
2. `nouveau-lx: rc: … mmuFault=0x… type=…` justo detrás de un «no señalizó».
   Es el nombre del fallo: `PTE`/`PDE` = tabla mal; `VA_LIMIT_VIOLATION` = la VA
   se sale; `UNSUPPORTED_APERTURE` = la apertura del PTE no vale para ese motor.
3. `nouveau-lx: BAR1 va=0x0 PD3[0] @…` — el recorrido de las tablas que RM ya
   construyó. Dice en qué nivel se corta la cadena y en qué apertura viven sus
   tablas, que es lo que hace falta para que la CPU escriba VRAM por la apertura
   en vez de por el CE.
4. `G6 — buffers VRAM listos (… rebote 1024 KiB)`: confirma que el rebote grande
   entró. Con él, una subida es un `LAUNCH_DMA` multilínea por MiB en vez de uno
   por página.

### Diseño del mapeo de BAR1 (números de ESTA placa)

Leído del host con `lspci -v -s 01:00.0` (no del guest, que no puede dimensionar
un BAR en caliente sin dejar de decodificar memoria):

| BAR | Tamaño | Base | Qué es |
|-----|--------|------|--------|
| 0 | 64 MiB | `0x8c000000` | registros (PRAMIN en +0x700000) |
| 1 | **16 GiB** | `0xa000000000` | apertura de FB — resizable, ya al máximo |
| 3 | 32 MiB | `0xa400000000` | instancia |

Cómo lo hace upstream (`nvkm/subdev/bar/gf100.c:96-121`): el vaspace de BAR1 es
un `nvkm_vmm` normal con rango `[0, bar_len)` —**el offset dentro de la apertura
ES la VA**— y para BAR1 **no** se llama a `nvkm_vmm_boot`: los niveles inferiores
se crean bajo demanda al mapear. Con GSP, `r535_bar_bar1_init`
(`bar/r535.c:111-131`) sólo sustituye la raíz del VMM por el PD3 que RM ya
construyó (`bar1PdeBase`) y deja que la maquinaria del driver cuelgue de ahí sus
propias tablas. Es decir: **construir nosotros PD0+SPT y enlazarlos bajo la
cadena de RM es lo que hace nouveau**, no un atajo.

Aritmética VER3 con estos 16 GiB (una entrada de cada nivel cubre):

| Nivel | Cubre por entrada | Índices que ocupa la apertura |
|-------|-------------------|------------------------------|
| PD3 | 2^47 | sólo el **0** — no hay entrada libre que tomar |
| PD2 | 2^38 = 256 GiB | sólo el **0** |
| PD1 | 2^29 = 512 MiB | **0..31** |
| PD0 | 2^21 = 2 MiB | 256 por PD1 |
| SPT | 4 KiB | 512 por PD0 |

De ahí sale el plan: la ventana propia va **al final de la apertura**
(`PD1[31]`), lejos del `PD1[0]` donde RM mapea lo suyo, así que la granularidad
de no-interferencia es de 512 MiB. Lo que el ciclo de placa tiene que confirmar
es sólo esto: que `PD3[0]`→`PD2[0]` existen y viven en VRAM (los de RM) y que
**`PD1[31]` está inválido**. Si estuviera ocupado, escribir ahí pisaría un mapeo
que RM usa y eso cuelga la tarjeta: en ese caso hay que buscar otra ventana, no
forzarla.

Por eso `run_bar1_probe` vuelca DOS caminos (offset 0 y la última ventana de
2 MiB): con uno solo no se decide nada y el ciclo cuesta un reinicio.

### Fallback (solo BAR0, no cierra G1 oficial)

Si VT-d no está disponible:

```bash
sudo ./scripts/l6-g1-vfio-noiommu.sh
```

Marca **PARTIAL** — valida lectura BAR0 pero sin aislamiento IOMMU.

## Veredicto (2026-07-29)

**G1 cerrado** en MSI Vector 16 HX con GB205. El port ha completado **G3b y
G4a–G5 en hardware real**: CE readback, QMD/PCAS SASS y `soso-llm run tiny` con
matvec en GPU (~97 OK). Operativa PCIe: cap Gen3 en FMC + bump Gen4/5 post-GSP
(**GO** 2026-07-30, `l6-g1-vfio-test.sh`); build con `SOSO_LXDDE`/`SOSO_QEMU_GPU`.
Detalle: `docs/L6-G3-nvkm-scope.md`; skill `soso-gpu`.

Generado como parte del roadmap L6 (lxdde).
