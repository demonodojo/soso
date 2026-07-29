---
name: soso-gpu
description: >-
  soso GPU nativa (NVIDIA nouveau/GSP, fase L6) — roadmap G1→G5, capa lxdde y
  port nvkm, firmware GSP, VFIO/IOMMU, shims de cabecera lx_emul y workflow de
  integración incremental. Úsala al tocar lxdde/ports/nouveau, kernel/src/lxdde,
  drivers GPU, scripts l6-*, firmware NVIDIA, o al avanzar los gates G1–G5.
---

# soso — GPU nativa NVIDIA (fase L6)

Objetivo: soso controla la GPU NVIDIA sin depender de Linux ni CUDA en el host.
Inferencia acelerada vía lxdde + nvkm + kernels SASS. Motor CPU (L1–L4) = fallback.

**GPUs soportadas (bring-up chip-aware en `gsp_bringup.c`):**
- **GB205 Blackwell** (`10de:2f18`, RTX 5070 Ti Mobile) — hardware primario, pero
  Blackwell es reciente y nouveau va por detrás.
- **Ampere GA10x** (RTX **3060**, GA106) — **objetivo de validación recomendado**:
  la ruta nvkm real (`ga102_gsp_new`) es Ampere y está **madura en nouveau 6.6**.
- Familia detectada por `NV_PMC_BOOT_0` (arch `>>20`) con fallback por device-id.

Docs fuente: `docs/L6-native-autonomy.md` (maestro), `docs/L6-G1-gate.md`,
`docs/L6-G3-nvkm-scope.md`, `docs/L6-H-cuda-hybrid.md`, y `PLAN-MODELOS-GRANDES.md`.

## Gates G1→G5

| Gate | Entregable | Criterio GO | Estado |
|------|-----------|-------------|--------|
| G1 | BAR0 bajo VFIO | log `NV_PMC_BOOT_0=0x…` | **GO** (2026-07-25 01:19): `NV_PMC_BOOT_0=0x1b5000a1` desde soso, GB205 real |
| G2 | firmware gb205 en rootfs | `lxdde-fw: cargado …/gsp/…` | **Go** (blobs .zst→.bin) |
| G2 | firmware gb205 + set ga102 (3060) | `lxdde-fw: cargado …/gsp/…` | **Go** |
| G3a | firmware ELF + GEM staging + fases | `N blobs GSP validados` | **Go** (soft boot) |
| G3b | GSP real vía nvkm (sin display) | `GSP booted` **sin** `soft` | **GO** (2026-07-25): `GSP booted (hw, GSP-FMC vía FSP)` en GB205 real, lockdown liberado, sin un solo all-ones en el log |
| G4a | RPC con GSP-RM (recibir + `SET_SYSTEM_INFO`/`SET_REGISTRY`) | `GSP_INIT_DONE` con `res=0x0` | **GO** (2026-07-25): llega tras 22 mensajes, `GSP-RM listo (RPC en marcha)` |
| G4b | RPC síncrono (`gsp_cmdq_call`) | round-trip + anillo que envuelve | **GO** (2026-07-25 hostcheck; ejercitado en HW vía G4c+) |
| G4c | Objetos de RM: cliente → device → subdevice (`GSP_RM_ALLOC`) | un `NV_RM_CONTROL` que responde | **GO** (2026-07-25): `objetos RM listos cli=0xc1d00000 dev=0xde1d0000 sub=0x5d1d0000` en GB205 real |
| G4d | VRAM + VA space + mapeos | reserva y mapeo verificados | **GO** HW (ejercitado por CE/compute 2026-07-29): VER3 + `SET_PAGE_DIRECTORY` + mapeos scratch/SASS |
| G4e | **Canal + CE** (`gsp_chan`, `gsp_ce`) | ALLOC GPFIFO/USERD/PB; copia CE + readback VRAM | **GO** (2026-07-29): `CE readback verificado (G4e GO)` en GB205; canal `0xca6f` + CE `0xcab5` |
| G4f | QMD + kernel SASS | `SYS_GPU_SUBMIT` con `on_gpu=1` real | **GO** (2026-07-29): PCAS 24 B + QMD v05; saxpy/matvec SASS en silicio (clase `0xcec0` sobre GR0) |
| G5 | LLM híbrido (capas en ~12 GiB VRAM) | matvec en GPU en `soso-llm` | **GO funcional** (2026-07-29): `soso-llm run tiny --max 4` → **97 matvec en GPU OK**, sin caída a CPU; tok/s vs CPU aún por medir en modelos grandes |
| **L6-H** | `--cuda-host` → cuda-proxy | texto + tok/s desde soso | **GO** (2026-07-27): ~35 tok/s, Docker llama-server |

**Pendiente post-G5 (no bloquea el gate):** medir tok/s nativo vs CPU en modelos
grandes; Ampere GA10x (3060) sin HW en esta máquina; enlace PCIe Gen4+ cuando
compute esté estable a Gen3.

### Estado en silicio (2026-07-29) — GB205 bajo VFIO

Criterio de ciclo verde en `target/g1-vfio-serial.log`:

- `lockdown liberado` (~+183 ms tras COT), `GSP-RM listo`, `CE readback verificado (G4e GO)`
- `matvec en GPU OK` (≈97 con `tiny --max 4`), **sin** `camino de GPU desactivado` / `RC_TRIGGERED` / `semáforo no llegó`
- `GSP-RM apagado (objetos=ok unload=ok halt=ok dma=off)`

**Operativa host (imprescindible tras cada reboot del host):** capar el enlace PCIe
del root port a Gen3 antes del ciclo VFIO — a Gen5 (32 GT/s) el FMC provoca tormenta
AER y a veces hard lockup:

```bash
sudo setpci -s 00:06.0 CAP_EXP+0x30.w=3:f          # Target Link Speed = Gen3
sudo setpci -s 00:06.0 CAP_EXP+0x10.w=20:20         # Retrain
# opcional Gen4 cuando compute esté estable: =4:f
```

Build con lxdde: `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build`
(o `SOSO_QEMU_GPU=vfio:…`, que ya activa la feature). Sin eso la imagen no enlaza
`liblxdde.a`.

### Gotchas cerrados el 2026-07-29 (compute)

1. **IMMD_DATA_METHOD** (`cp_pb_immd` en `gsp_compute.c`): el dato va en bits 28:16
   de la cabecera, **sin** dword detrás. G4h lo había roto (literal `1` en el campo +
   dato suelto) → `PBDMA_ERROR` type 32 con `GPGet=0`. Pushbuffer PCAS = **24 B**
   (SET_OBJECT + WFI immd + SEND_PCAS_A + PCAS2 immd). Hostcheck: `SEND_PCAS (24 B)`.
2. **USERD GPGet en Blackwell no escribe** (nouveau `862450a` / Skeggs): desde Volta
   solo a timer; en `BLACKWELL_CHANNEL_GPFIFO_*` el writeback desapareció. Leer
   `USERD+0x88` siempre da 0 aunque el trabajo corra. Progreso = semáforo CE/QMD →
   `gsp_chan_ack_progress()` (`gpget` SW). `pb_rewind` mira `gpget==gpput`, no el USERD.
3. **Anillo GPFIFO y Put sin módulo**: con 64 entradas, `GPPut=64` cuelga; con
   4096, `GPPut=4096` dejaba `sem=0` (`gpput=4096`). Upstream no escribe
   `USERD.GPGet` (Blackwell sin writeback; progreso = semáforo). Fix: en
   `submit`, `USERD.GPPut = gpput % ENTRIES`; `ack_progress` solo avanza
   `gpget` SW. GPFIFO = **4096×8 B**; VAs GPFIFO → PB → notifier.
4. **RC_TRIGGERED**: `gsp_rpc_rc_triggered_log` vuelca 8 palabras del journal
   (cabeza) además de type/chid/engn.
5. **Enlace PCIe Gen5**: inestable durante reset FMC bajo VFIO → cap Gen3 (arriba).

**G1 superado (2026-07-25).** Con VT-d activo en la BIOS y el bind persistente puesto
(`l6-g1-vfio-persist.sh --enable` + reboot), `sudo ./scripts/l6-g1-vfio-test.sh` da GO:
soso lee `NV_PMC_BOOT_0=0x1b5000a1` de la BAR0 real (`familia=Blackwell (gb20x)`).
El script no imprime nada durante los ~90s de `cargo xtask run` (todo va a
`target/g1-vfio-serial.log`): **parece colgado y no lo está**.

**`fw_loading` no era falta de firmware: era orden de arranque.** El primer G1 real dio
`lxdde-fw: no encontrado /lib/firmware/nvidia/gb205/gsp/…` → `GSP init falló
(status=fw_loading)` **con los blobs ya dentro de la imagen** (`sosofs: 61 inodes,
31843/32768 bloques`, 47 ficheros = todo `rootfs/`). La pista está en el orden del log:
el bring-up GSP imprimía en la línea 492 y `fs: sosofs montado` en la 502 —
`lx_request_firmware` resuelve por VFS (`kernel/src/lxdde/firmware.rs`) y aún no había
nada montado. Arreglado subiendo `fs::init()` por encima del bloque `lxdde` en
`kernel/src/main.rs` (queda tras los drivers de bloque y antes de `net::init()`;
`nvidia_compute::init` sigue después de lxdde porque consulta `gsp_ready()`).
Fallback sin IOMMU (solo BAR0, PARTIAL): `l6-g1-vfio-noiommu.sh`.

**Firmware chip-aware (2026-07-25).** `gsp_fw.c` tiene una tabla por familia:
Blackwell = `gb205/{bootloader,fmc,gsp}`, Ampere = `ga102/{bootloader,booter_load,
booter_unload,gsp}`; `gsp_fw_load_all(chip)` exige todos los blobs de su juego. Antes
cargaba los dos juegos y el ucode `gsp-570.144.bin` (60,6 MiB) se duplicaba —
en linux-firmware el de gb205 es un **symlink** al de ga102. El heap del kernel está
en 256 MiB (`kernel/src/mm/heap.rs`); con los dos juegos ni 256 ni 512 llegaban.
`lx_release_firmware` ya libera de verdad (era no-op). Desde el paso de radix3 el
ucode **no** se copia a un GEM y su blob en bruto se suelta con
`gsp_fw_release_one(GSP_FW_UCODE)` en cuanto `gsp_rm_prepare` tiene su copia: pico
≈125 MiB en vez de ≈190. La ruta Ampere está escrita pero **sin probar**: no hay
3060 en esta máquina.

**Ruta por familia en el bring-up.** `gsp_bringup.c` bifurca: Ampere → `run_acr_sec2()`
(el ACR de siempre); Blackwell → `run_fmc_blackwell()`. El ACR de `acr_fw.c` es de
Ampere (ucode `ga102` en SEC2) y en GB205 daba `falcon boot mbox0=0xbadf4100` — el
falcon ni ejecuta — porque GB20x arranca por GSP-FMC/FSP.

**GSP-FMC (`fmc_lx.c`), estado real.** Upstream de referencia (NO está en el árbol
6.6 pinneado; se consultó en git.kernel.org): `nvkm/subdev/fsp/{gh100,gb202}.c`,
`nvkm/subdev/gsp/gh100.c`, `include/nvhw/ref/{gh100,gb202}/dev_{fsp_pri,therm}.h`.
Datos que ya están verificados contra el blob de esta máquina:
- El ELF `fmc-570.144.bin` cumple la cabecera fija de upstream, 6 secciones, y sus
  CRC (`sh_info`) cuadran. Secciones: `hash`=48 B, `signature`=96, `publickey`=97,
  `image`=199240. Esos 48/97/96 son los tamaños COT de **`gb202_fsp`** (gh100 usa
  384/384) → GB205 va por la variante gb202, `cot.version = 2`.
- Registros (dentro de los 16 MiB de BAR0 mapeados): `NV_THERM_I2CS_SCRATCH` en
  **0x00ad00bc** para gb202 (¡no el 0x000200bc de gh100!), éxito = `0xff`;
  `NV_PFSP_QUEUE_HEAD/TAIL(0)` = 0x008f2c00/0x008f2c04; `MSGQ_HEAD/TAIL(0)` =
  0x008f2c80/0x008f2c84.
- **Leído en HW desde soso (2026-07-25)**: `FSP secure boot=0x000000ff (completo)` y
  las cuatro colas a 0 → el FSP terminó su arranque seguro, el canal EMEM está vivo y
  ocioso, y la offset gb202 es la buena (con la de gh100 no saldría justo `0xff`).
  El receptor del COT está listo y verificado; falta construir el payload.

**La cadena entera (pasos 1–6) está escrita.** Falta validar el paso 6 en hardware:
es el primero que escribe registros de la GPU y hasta ahora solo se ha probado
contra el FSP simulado del hostcheck.

**radix3 (`gsp_rm.c`), hecho.** Primero de los tres bloques que faltaban. El ucode
`gsp-570.144.bin` es un ELF64 REL de RISC-V: `.fwimage` = 0x3c99000 B (~60,5 MiB, ya
múltiplo de página) + una firma de 4 KiB por familia (`.fwsignature_gb20x` /
`.fwsignature_ga10x`). `gsp_rm_prepare()` extrae las dos, copia la imagen a un buffer
alineado a página y construye la tabla de 3 niveles igual que `nvkm_gsp_radix3_sg`
(`nvkm/subdev/gsp/r535.c`): hojas = física de cada página de la imagen, así que el
firmware no tiene que ser contiguo. En gb205 salen 15513 páginas → hoja 126976 B,
nivel 1 y raíz 4096 B. `radix3_verify()` releé los tres niveles antes de que el GSP
los vea. Apoyos nuevos en `kernel/src/lxdde/mem.rs`: `lx_alloc_pages_exact`
(`lx_kmalloc` solo alinea a 8) y `lx_virt_to_phys`. Fase serial: `rm_radix3`, log
`radix3 verificada raíz=0x…`. **Sigue sin tocar un registro.**

**WPR meta (`gsp_wpr.c`), hecho.** Paso 4. Referencia `gh100_gsp_wpr_meta_init` +
`rm/r570/nvrm/gsp.h` (el `nvkm_gsp_fwif` de gh100 dice literalmente `"570.144"`, la
misma versión que los blobs de aquí). **En la ruta FMC el driver NO calcula
direcciones de WPR** (`offset_set_by_acr` en `r570_wpr_libos3_gb20x`): solo punteros
a sysmem y tamaños, y `wpr_meta_verify()` **falla si `gspFwWprStart`/`gspFwOffset`/
`frtsOffset`/`fbSize`… vienen escritos**. Estructura de 256 B exactos (asserts de
compilación sobre `sizeof` y offsets). El bootloader es `nvfw_bin_hdr` +
`RM_RISCV_UCODE_DESC`: en el blob real `data_offset=0x6c`+`data_size=0x31000` = el
fichero exacto, `version=5`, manifest 0..0xa00, datos 0xa00..0xb200, código
0xb200..0x30a00. **VRAM real** por `0x1183a4` (MiB, `ga102_fb_vidmem_size`, vale
hasta Blackwell) — sustituye a la heurística por SKU, que queda de respaldo; con
12288 MiB el heap sale 22+14+2+96 = **134 MiB**. Fase `wpr_meta`.

**libos boot args (`gsp_libos.c`), hecho.** Paso 5. Referencias
`r535_gsp_libos_init`, `r535_gsp_shared_init`, `r570_gsp_set_rmargs`. **Trampa de
versión: r535 y r570 NO comparten el layout** de `GSP_ARGUMENTS_CACHED` ni de
`MESSAGE_QUEUE_INIT_ARGUMENTS` — r570 quita `locklessCmd/StatQueueOffset` y añade
`bDmemStack`; con 570.144 va el de r570. Cuatro piezas: memoria compartida (un
bloque contiguo, tabla de PTEs + 2 colas de 256 KiB → **129 PTEs**, 63 mensajes de
4 KiB por cola), tres logs de 64 KiB con su lista de PTEs dentro del propio búfer
tras el *put pointer*, `RMARGS`, y la página de 4 regiones
`LibosMemoryRegionInitArgument` con el nombre en `u64` big-endian (`"LOGINIT"` =
`0x4c4f47494e4954`). Encima, `GSP_FMC_BOOT_PARAMS` enlaza WPR meta (*coherent*) y
libos (*noncoherent*); `wprCarveout` a cero, lo talla el FMC. Siete asserts de
compilación fijan los layouts. `fmc_lx_stage()` copia la imagen FMC y su cadena de
firma a memoria coherente (el FSP las lee él). Fases `libos_args` → `cot_ready`.

**COT (`fsp_lx.c`), escrito — paso 6, el primero que escribe MMIO.** Referencias
`gh100_fsp_boot_gsp_fmc`, `gh100_fsp_{send,recv,poll,wait}`, `gp102_flcn_emem_pio`,
`gh100_gsp_lockdown_released`. Mensaje MCTP/NVDM de **868 B**: `0xc0000000`
(SOM|EOM) + `0x1410de7e` (vendor-PCI 0x7e / 0x10de / NVDM 0x14=COT) + payload packed
de 860 B. Los campos de firma miden 384 B aunque gb20x llene 48/97/96.
`frtsVidmemOffset` es offset **desde el final de la VRAM** = `ALIGN(nonWprHeap +
pmuReserved, 2 MiB)` = 0x1C00000. Transporte: EMEM del falcon FSP (`0x8f2000`,
puerto/dato `0xac0`/`0xac4`, autoinc bit 24 escritura / 25 lectura); `QUEUE_TAIL` =
tamaño−4 y `QUEUE_HEAD` = 0 es el timbre. Luego se sondea `MAILBOX0` del falcon GSP
(`0x110000`+0x40) hasta que deje de valer `0xbadf41xx` y caiga `HWCFG2` bit 13
(`RISCV_BR_PRIV_LOCKDOWN`). **Guardas antes de escribir**: secure boot `0xff`, colas
ociosas, payload completo, firma 48/97/96. Todas las esperas acotadas (1 s / 1 s /
4 s); cualquier fallo → −1 y sigue a `booted (soft)`. Log de éxito:
`GSP booted (hw, GSP-FMC vía FSP)`.

**Gotcha que se comió la primera prueba en HW (2026-07-25): `lx_mdelay` no
esperaba.** `timer::sleep_ms` registraba un timer y llamaba a
`fiber::block_current()`, pero `yield_now()` **retorna en el acto si solo hay una
fibra** — que es justo el caso durante el bring-up de la GPU. Resultado: los bucles
de sondeo daban sus miles de vueltas en microsegundos. El primer COT real fue
aceptado por el FSP y aun así dio `GSP-FMC no arrancó a tiempo` porque los "4 s" de
espera del lockdown fueron ~16 ms. Arreglado en `kernel/src/lxdde/timer.rs`:
`sleep_ms` gira sobre `pit::uptime_ms()` cediendo a otras fibras si las hay, con
detección única de reloj parado (`CLOCK_DEAD`) para no colgar el arranque si el PIT
no avanza. Afectaba también a `gsp_mmio_poll_ready`. **Si un poll de este port
"falla instantáneamente", sospecha del reloj antes que del hardware.**

**Gotcha 2 (2026-07-25): all-ones NO es "listo", es la GPU fuera del bus.**
`gsp_mmio_poll_ready` daba por bueno `0x118128=0xffffffff` / `0x118234=0xffffffff`
porque cumplen `(a&1) && ((b&0xff)==0xff)` — y así el checklist dio **`G3b hw boot
GO` con la tarjeta muerta**. Ahora `gsp_mmio_alive()` trata el all-ones como
silencio del bus, el bucle del lockdown aborta con mensaje propio, y `g3-check`
degrada el criterio a PEND si ve la huella (`fuera del bus`, `118128=0xffffffff`,
`boot0=0xffffffff`). **Antes de creerte un GO, mira que los registros del log no
sean todo efes.**

**Paso 6 cerrado en HW (2026-07-25): `GSP booted (hw, GSP-FMC vía FSP)`.** El FSP
acepta el COT, el FMC arranca y baja el lockdown: `HWCFG2` pasa de `0x8187a7f7` a
`0x818787f7` (cae el bit 13) con `mbox0=0`. Sin un solo `0xffffffff` en el log.

**Gotcha 8 (2026-07-28): "se cayó del bus" puede ser un reset del enlace, y
rendirse en 3 ms lo hace indistinguible de la muerte.** Ciclo VFIO con la carga
por SSH ya visible: el FSP acepta el COT, y **+3 ms** después todo el MMIO se lee
a unos → `GSP init falló (status=gone)`. Pero el monitor del root port del propio
script (`target/g1-vfio-link.log`) enseña el enlace haciendo **32 GT/s → 2.5 GT/s
→ 32 GT/s en 207 ms**, dos veces (la segunda al cerrar QEMU el fd de vfio). Una
tarjeta muerta de las de 2026-07-25/27 no reentrena el enlace: esto tiene pinta de
**reset**, no de muerte. `fsp_gone_recovered` (`fsp_lx.c`) ya no se rinde en la
primera lectura: espera hasta `FSP_GONE_BUDGET_MS` (2 s, ~10× los 207 ms medidos)
sondeando `gsp_mmio_alive()` y reintentando el decode por configuración cada
250 ms, y descuenta lo esperado del plazo del bucle para que los `+N ms` de las
trazas no mientan. Cubierto sin HW en el hostcheck (`fake_die_after_mbox0` +
`fake_revive_after_mdelays`): un reset de 300 ms tiene que **arrancar**, y una
muerte sin vuelta tiene que seguir fallando.

**Y el corolario del espacio de configuración: bajo VFIO un `id` válido no prueba
nada.** En ese fallo `gsp_mmio_pci_recover()` leyó `id=0x2f1810de` —correcto— con
`sts=0xffff` y `cmd=0xfbff`: vfio-pci **emula** los primeros registros desde la
copia que guardó al abrir el dispositivo, así que el id sobrevive a la muerte de
la tarjeta. El juez es `gsp_mmio_alive()`, que lee un registro de verdad; el
`sts` a unos se traza aparte para que el log no invite a creer que está sana.

**Antes de eso hubo un intento en el que la GPU se caía del bus** ~1 s después de
arrancar el FMC, con `nvidia.ko` cargándose y descargándose ~5 veces por segundo
mientras la tarjeta estaba en VFIO (pares `nvlink: Nvlink Core is being initialized`
/ `Unregistered` cada ~180 ms en `dmesg`). Se atribuyó a `nvidia-persistenced` y se
enmascaró; **la atribución era errónea** (2026-07-25).

**El culpable real: un bucle udev↔modprobe que acaba colgando el host.**
`/usr/lib/udev/rules.d/71-nvidia.rules` lanza `modprobe nvidia_{modeset,drm,uvm}` al
ver el `add` de `/bus/pci/drivers/nvidia`. Con la dGPU en vfio-pci el probe falla,
`nvidia` se descarga, desaparece el directorio, y el siguiente `modprobe` lo recrea
→ ciclo infinito. Enmascarar `persistenced` quitaba **una** de las cuatro acciones
`RUN+=`, así que el bucle seguía: acumuló 85 100 ciclos en 4 h, ahogó el journal y
hubo que resetear a mano a mitad de una prueba. Lo corta `install <mod> /bin/false`
(ya lo genera `--enable`); en caliente, esas líneas + `udevadm control --reload` +
`rmmod nvidia`. **Al medir justo después el ritmo se dispara (327/min → 14 000/min):
es la cola de udev vaciándose porque ahora cada `modprobe` falla al instante, no el
bucle empeorando.** Espera ~2 min y verás 0. Mantener `persistenced` enmascarado
sigue siendo buena idea (no puede funcionar sin GPU), pero no es lo que arregla esto.

**Pila RPC (`gsp_rpc.c`), primer tramo: recibir.** Con el GSP arrancado, GSP-RM
habla por las colas del paso 5. Dos cabeceras por elemento: `r535_gsp_msg` (48 B) y
`nvfw_gsp_rpc` (32 B, con `length`/`function`/`rpc_result`), ambas con assert de
tamaño. **Lo que se presta a error son los punteros cruzados**: el `writePtr` de la
cola de mensajes está en su cabecera `tx` (lo escribe el GSP) pero su `readPtr`
está en la cabecera `rx` de la **cola de comandos** (lo escribimos nosotros); y las
entradas empiezan **detrás** de la primera página del anillo. Se avanza
`ceil((length + 48) / 4096)` páginas módulo 63. Antes de escuchar hay que publicar
`app_version` en `0x110080` y comprobar el bit 7 de `NV_PRISCV_RISCV_CPUCTL`
(`0x111388`, `ga102_flcn_riscv_active`) — en el arranque que cerró G3b ya valía
`0x180`. Hito: ver llegar **`GSP_INIT_DONE`** (evento `0x1001`).

**Probado en HW (2026-07-25): el transporte funciona, el contenido no.** GSP-RM
manda cientos de RPCs bien formados; llegan `GSP_POST_NOCAT_RECORD` (0x1020, el
catálogo de crashes de RM) en avalancha, `GSP_LOCKDOWN_NOTICE`, `UCODE_LIBOS_PRINT`
y al final `GSP_INIT_DONE` con **`rpc_result=0x59` = `NV_ERR_OPERATING_SYSTEM`**.
**Causa:** `r535_gsp_oneinit` encola `GSP_SET_SYSTEM_INFO` y `SET_REGISTRY` en la
cmdq **antes de arrancar el GSP**; GSP-RM las consume durante su init. Sin ellas
arranca a ciegas. Siguiente paso: el envío por la cmdq.

**G4b: llamada síncrona (`gsp_cmdq_call`), hecha.** `send` + `gsp_rpc_recv`, que
empareja la respuesta **por `function`** — no por secuencia: la `sequence` de la
cabecera RPC va a 0 también upstream, la que se incrementa por mensaje es la del
*elemento* de cola. Dos casos que `GSP_INIT_DONE` (32 B, sin payload) no ejercitaba y
ahora cubre el hostcheck: copiar el payload de la respuesta, y **un mensaje de varias
páginas que da la vuelta al anillo** — con un solo `memcpy` se leía pasado el final
del área de entradas (bug latente, nunca disparado). La copia se hace **antes** de
publicar el puntero de lectura: en cuanto se publica, el GSP puede reutilizar esas
páginas.

**Gotcha 3 (2026-07-25): `set_boot0` borraba el éxito del bring-up.**
`lx_nouveau_set_boot0` ponía `g_phase = GSP_BAR0` sin condiciones, y
`nvidia_probe::init()` corre **después** del bring-up (`main.rs`: `lxdde::init` →
`gpu::init` → `nvidia_probe::init`). Resultado: el arranque llegaba a `rm_ready` y el
log decía `GSP=bar0`, `gsp_ready()` daba falso y el cómputo se iba a la CPU sin
avisar. Ahora solo asciende desde `GSP_NONE`. Además `lx_nouveau_gsp_ready()` no
incluía `GSP_RM_READY` — el estado *mejor* se reportaba como "no listo".

**El valor de `on_gpu` era una mentira y ahora no.** `lx_nouveau_submit_saxpy`
devolvía 1 (= "lo calculó la GPU") con solo estar el GSP arrancado, mientras por
debajo corría un bucle de CPU. Eso hace el criterio GO de G4 incumplible de fallar —
el mismo vicio que dio `G3b GO` con la tarjeta fuera del bus. Devuelve 0 hasta que
haya canal y el resultado venga de VRAM.

**El toolchain de SASS ya no bloquea G4f** (lo bloqueó hasta el 2026-07-28, cuando
`saxpy.sass.bin` medía 0 bytes y no había `ptxas` en la máquina). Hoy los dos blobs
están compilados y embebidos: saxpy 512 B / 32 instrucciones, matvec 2944 B / 184, y
el hostcheck compara el `.bin` con el embebido. **G4e/G4f/G5 están en GO en GB205**
(2026-07-29): CE readback + PCAS/QMD + `soso-llm` con matvec en GPU. El historial de
diagnóstico de `GPGet=0` / `NO_MEMORY` GR0 que sigue debajo documenta el camino hasta
ese GO; no es el estado actual.

**Primer ciclo de G4e en HW (2026-07-28): RM acepta todo y el host no recoge
nada.** Con la carga por SSH y VFIO: `GET_CLASSLIST_V2` → canal `BLACKWELL_CHANNEL_
GPFIFO_B` (0xca6f) y CE `BLACKWELL_DMA_COPY_B` (0xcab5) reservados sobre el vaspace
externo, `BIND(motor 9)` + `SCHEDULE(bEnable=1)` + `GET_WORK_SUBMIT_TOKEN` = 0
(runlist 0, chid 0), los cinco búferes mapeados. Y luego **`GPGet=0`, `Get=0`,
semáforo a 0**: el trabajo se encoló (`GPPut=1`, `Put=100`, entrada bien formada
apuntando al pushbuffer) y el host no lo tocó. Detrás, **dos `GSP_POST_NOCAT_RECORD`**
y el canal de GR0 fallando con `NO_MEMORY` (0x51).

**Gotcha 9 (2026-07-28): `SET_OBJECT` no cabe en un inmediato.** El pushbuffer
empezaba con `pb_immed(..., NVC56F_SET_OBJECT, ce->handle)` y el campo `IMMD_DATA` es
**28:16, trece bits**: `0xc6b50000 & 0x1fff` = 0, o sea la subchannel 0 atada al
objeto **0**, y todos los métodos del CE que venían detrás sin motor que los
ejecutase. Upstream lo emite como método normal de dos palabras
(`PUSH_MTHD(push, NV9039, SET_OBJECT, handle)` con `PUSH_WAIT(push, 2)`). Estaba
igual de mal en el QMD de compute (`NVCDC0_SET_OBJECT`). El hostcheck buscaba el
`LAUNCH_DMA` —que era perfecto— y no miraba la primera palabra; ahora comprueba la
cabecera bit a bit y que el dato sea el handle entero. **Un pushbuffer con un método
mal no se ve distinto de un doorbell que no llega: los dos dejan `GPGet=0`.**

**Segundo ciclo (2026-07-28): el `SET_OBJECT` estaba mal y NO era la causa.** Con el
método bien (26 dwords, `Put=104`) el resultado es idéntico: `GPGet=0`. Lo decisivo es
que `gsp_rpc_drain` no encontró **ni un evento** después: RM no se quejó, así que el
trabajo no llegó a fallar del lado de RM — el host no lo recogió. Un error de método
habría dejado un aviso.

**Gotcha 12 (2026-07-29): el indicador de "firmware arrancado" NO es el mismo en
Blackwell, y creerlo cuesta un ciclo.** Con la comprobación nueva puesta salió
`GFW boot NO completado … 118234=0x00000000 progress=0x00` **en una máquina recién
reiniciada y con una FLR hecha justo antes** — o sea que el registro vale 0 siempre. La
fuente lo explica: `0x118234` es
`NV_PGC6_AON_SECURE_SCRATCH_GROUP_05(0)` alias `..._GFW_BOOT` (PROGRESS 7:0, COMPLETED
0xff) **de `dev_gc6_island_addendum.h` de tu102**, y para gb202 NVIDIA **no publica ni
`dev_gc6_island.h`** (su directorio sólo tiene dev_boot, dev_ce, dev_fault, dev_fsp_*,
dev_mmu, dev_ram, dev_runlist, dev_therm*, dev_vm, dev_xtl_ep_pcfg_gpu…). En gb202 el
indicador es **`NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE`** (= `NV_THERM_I2CS_SCRATCH`,
`0x00ad00bc`; STATUS 31:0, SUCCESS `0xff`, FAILED `0x00`) — el mismo registro que este
port ya leía como "FSP secure boot" y que ya vale `0xff`. Su nombre de verdad es ése.

Y el otro registro de la pareja tampoco era lo que parecía: `0x118128` es
`..._GROUP_05_PRIV_LEVEL_MASK`, cuyo bit 0 es `READ_PROTECTION_LEVEL0_ENABLE` — un
**permiso de lectura**, no un progreso.

**Consecuencias, todas correcciones de lo que escribí ayer:**
- La puerta que añadí antes del COT **estaba de más y era falsa**: `fsp_ready_to_send()`
  ya exige `FSP_BOOT_COMPLETE` desde antes y pasaba. Quitada; sólo bloqueó una tarjeta
  sana y dejó el arranque en `booted_soft`.
- `gsp_mmio_gfw_wait` sólo se llama **si la familia no es Blackwell** (en gb20x gastaba
  2 s esperando un registro que no existe) y su mensaje ya no dictamina "el devinit no ha
  terminado": es informativo.
- **La hipótesis del devinit está muerta**: el arranque del firmware estaba completo en
  todos los ciclos. El `ASSERT` de RM sobre `GFW_BOOT_PROGRESS` es RM mirando ese mismo
  scratch de Turing, y no le impidió llegar a `GSP_INIT_DONE`, objetos, canal y CE — es
  ruido, no la causa.
- La FLR previa del script **se queda** (llegar con la tarjeta en un estado conocido es
  buena higiene y el ciclo del 2026-07-29 sobrevivió con ella), pero **no** por la razón
  que decía: no rehace ningún devinit que faltara.

**Y el `0xbadf5040` de la runlist sigue abierto, con mejor pista: PTOP confirma que
`data[11]` SÍ era la base.** Leída del chip: GR0 → `0xd00000`, CE0 → `0xd00000`, CE1 →
`0xd00400`, 32 motores en 152 palabras (GSP0 en `0x110000`, que casa con la base del
falcon que ya usábamos). Idénticos a `engineData[11]`. Así que la dirección era correcta
y lo que falla es el **acceso**: un `0xbadfxxxx` con el resto del chip respondiendo
apunta a que ese bloque PRI no es legible con nuestro nivel de privilegio bajo VFIO — no
a que esté sin inicializar. Si es eso, `chan_dump_hw` nunca podrá leer el estado del
canal en este chip y hay que buscarlo por RM.

**Gotcha 10 (2026-07-28): nadie esperaba el devinit de la GPU, y RM lo dijo por
NOCAT.** Los dos primeros registros NOCAT, volcados ya, contienen literalmente
`ASSERT` + `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT_PROGRESS_N`: GSP-RM asertó
que el **GFW boot** —el firmware de la propia GPU, el que corre el devinit tras un
reset— no está en COMPLETED. Y es verdad que no lo mirábamos: `gsp_mmio_poll_ready`
(que comprueba justo eso, `0x118128` bit 0 y `0x118234 & 0xff == 0xff`, como
`tu102_devinit_wait`) **sólo se llama en la ruta ACR de Ampere**; la ruta FMC de
Blackwell iba directa al COT. Un FLR de vfio es un reset, y tras un reset ese devinit
vuelve a correr. Ahora `gsp_mmio_gfw_wait` (2050 ms, los de upstream) se llama **dos
veces** —tras mapear BAR0 y otra vez tras el FMC, porque en medio hay un reset del
enlace (gotcha 8)— y deja los dos valores en el log pase lo que pase. **Si el devinit
no ha terminado, no hay FIFO que recoja un doorbell**, y eso explicaría el `GPGet=0`,
el `NO_MEMORY` de GR0 y los `0xbadf` de abajo con una sola causa.

**Y el `0xbadfxxxx` no es un dato, es el anillo PRI diciendo que ahí no hay nadie.**
El volcado de registros del canal leyó `0xd00000+0x100 = 0xbadf5040` y
`+0x004/+0x008 = 0x00000002` los dos: ni es un `chcfg` (daría chram y nº de canales,
salió chram=0 y "4 canales") ni responde. Así que **`data[11]` no es la base de la
runlist en GB205** —o el bloque no está inicializado— y la ruta queda descartada: la
comprobación cruzada (`chcfg & 0xfffffff0` == `data[14]`) la cazó y el volcado se para
ahí en vez de inventarse un estado. `gsp_mmio_pri_error()` reconoce esa familia de
valores (el `0xbadf4100` del falcon con el ACR de Ampere era la misma cosa).

**Y para no volver a gastar un ciclo adivinando, cuatro diagnósticos nuevos:**
- **Los NOCAT se vuelcan** (`gsp_rpc.c`): palabras en crudo + las cadenas que lleven
  dentro. RM mete ahí el motor, el código de error y la aserción; contarlos no dice
  nada. Se **desduplican por contenido** (huella FNV-1a saltando la palabra 2, que es
  un contador de tiempo) y el tope son 6 avisos *distintos*: volcar "los dos primeros"
  gastó el cupo en dos copias del mismo aviso y tiró los tres siguientes, entre ellos
  el que acompañaba al `NO_MEMORY` de GR0.
- **La sonda del aperture de usermode** (`chan_probe_usermode`): el doorbell es una
  escritura ciega, así que un aperture en la dirección equivocada se ve igual que un
  canal que no arranca. Lo que sí se puede leer es el reloj — `clc361.h` pone
  TIME_0/TIME_1 en usermode+0x80/+0x84 y el mismo reloj está en `NV04_PTIMER_TIME_0`
  (0x9400) — así que si el privilegiado avanza y el de usermode no, el `+0x90` no es el
  timbre de nadie. Se sondea al arrancar el canal, antes del primer submit. El
  hostcheck exige que el port lea ese offset (mismo truco que `FAKE_DOORBELL_REG`).
- **`gsp_rpc_drain`**: los avisos de RM sobre el canal son eventos, y sin nadie
  escuchando se quedan en la cola — los dos NOCAT del CE aparecieron páginas más
  abajo, dentro del alloc siguiente, como si fueran de aquél. Se llama tras un CE que
  no señaliza. Que **no** haya nada también informa: RM no se quejó → el trabajo no
  llegó a fallar en RM, no arrancó.
- **`chan_dump_hw`** (`gsp_chan.c`): registros del FIFO leídos de BAR0 —`chcfg`/`dbcfg`
  de la runlist, el estado del canal en la channel RAM (`2`=corriendo, `3`=parado,
  `ffffffff`=sin atar) y los INTR del PBDMA, donde `EMPTY_SUBC` (0x00800000) e
  `ILLEGAL_MTHD` (0x00200000) señalan justo el bug de arriba con su subc y su método.
  Las bases salen de la tabla del FIFO (`data[11]` runlist, `data[14]` channel RAM,
  índices inferidos) y **se validan contra el silicio**: `chcfg & 0xfffffff0` tiene que
  ser `data[14]`, y si no cuadra el volcado lo dice y no usa nada. Un 0 en un INTR no
  prueba nada (RM los limpia al atenderlos); un bit puesto, sí. **En GB205 la
  validación falla** (ver el `0xbadf5040` de arriba): esta ruta no da estado del canal
  en este chip, sólo dice que ese bloque no contesta — que ya es un dato.
- **La espera del GFW boot** (`gsp_mmio_gfw_wait`, gotcha 10): en dos puntos del
  arranque, con los valores crudos en el log.
- **PTOP** (`gsp_top.c`), que sustituye a la inferencia: la tabla de motores que
  publica el chip en `0x0224fc >> 20` palabras desde `0x022800`, con tipo, bloque de
  registros, **runlist**, id de fallo y bit de reset por motor, parseada como
  `ga100_top_parse` (tres palabras encadenadas por el bit 31, huecos incluidos). El
  volcado del canal pide la runlist a PTOP y **enseña las dos** —la de PTOP y la de la
  tabla de RM— para zanjar si `data[11]` significaba lo que creíamos. Cubierto en el
  hostcheck con tres motores de runlists distintas, un hueco en medio, y el caso de
  GPU ausente (que no debe "encontrar" tabla).

**La fase del bring-up llega a userspace** (`GpuInfo.phase`, 16 B con NUL). Se rellena
sólo para la NVIDIA —un `rm_ce` sobre el dispositivo de software sería mentira— y sale
por `soso-llm` («dispositivo «…» (fase rm_ce)» y, si nada se calculó en el silicio, una
línea que lo dice) y por la sonda de `init test`, que antes mandaba a abrir el log de
serie para averiguar una palabra. Va como **cadena y no como código**: la tabla de
nombres vive sólo en el port, porque dos tablas de lo mismo divergen siempre.

Y `cargo xtask g3-check` ya evalúa los criterios nuevos —los dos GFW boot, PTOP y el
aperture de usermode— y avisa si ve un NOCAT de `GFW_BOOT_PROGRESS`, así que tras cada
ciclo dice el estado sin leer 900 líneas de serie.

**Gotcha 11 (2026-07-28): el chid se pide por los índices de USERD, y los teníamos
clavados a cero.** El `NO_MEMORY` del canal de GR0 tiene una explicación que estaba
**escrita en nuestro propio header y que el código no seguía**: en `r535_chan_alloc` el
chid lo elige el llamante y viaja hasta RM sólo dentro de dos subcampos de `flags`
—`CHANNEL_USERD_INDEX_VALUE = chid % 8` y `..._PAGE_VALUE = chid / 8`, con
`PAGE_FIXED = TRUE` e `INDEX_FIXED = FALSE`—. `chan_fill_alloc` los ponía a 0 y 0
fijos: con un canal da igual (chid 0 son ceros, y el token confirmó chid 0), pero el
segundo pedía **el mismo slot que el primero** declarándolo fijo. Ahora salen de
`c->chid = idx`, y el port **contrasta** el chid del `GET_WORK_SUBMIT_TOKEN` con el
pedido: si no cuadran, el chid no se pide por ahí y hay que mirar en otro sitio. El
hostcheck exige que el segundo canal declare otro índice y reciba otro token —dos
canales con el mismo token serían dos canales pateando el mismo—.

**Contexto de GR (`gsp_grctx.c`), escrito: sin promocionarlo el QMD no puede correr.**
En el camino GSP un canal de GR no ejecuta nada hasta que su contexto está
promocionado. La cadena, en el orden de `r535_gr_oneinit` (canal → clase → promoción,
que es el que sigue el bring-up):
1. `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO` (0x20800a32, params
   de **1664 B** = 8 motores × 26 propiedades × {size, alignment}) da los tamaños.
2. `gsp_grctx_plan` los convierte en la lista de búferes con el mapa de
   `r535_gr_get_ctxbuf_info`. **Tres reglas que no se ven si se equivocan**: el
   principal se lleva `ALIGN(size, 0x1000) + 64 páginas`; la página es 2^21/2^16/2^12
   según el tamaño; y el attribute CB se alinea a `order_base_2(size)` —**no** a su
   página—, así que 24 MiB pide 32 MiB de alineación y no 2 MiB. Un tamaño 0 se salta,
   y el `PRIV_ACCESS_MAP` se promociona **dos veces** (la segunda como
   `UNRESTRICTED`). Todo eso está en el hostcheck con los números a mano.
3. `gsp_grctx_promote` reserva en VRAM, mapea en el vaspace (ventana propia en
   `GSP_VA_BASE + 0x40000000`) y manda `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (0x2080012b).
   `engineType` es el **1 literal** del control (GR), no el del canal; `hClient`/`ChID`/
   `hVirtMemory`/`virtAddress`/`size` van a cero como upstream. Se reserva **todo**,
   globales incluidos: somos el primer canal y no hay contexto dorado del que heredar
   (el camino `golden = true`).

El hostcheck cubre las dos mitades por separado, que es lo que hace falta: `check_grctx`
la aritmética del plan (números a mano) y `check_g4e_chan_ce` **los bytes de la petición
tal como viajan** por la cmdq —cmd, objeto, `paramsSize` 560, los campos a cero, el
`entryCount`, que la entrada del principal lleve física y `physAttr=4`, que el
`PRIV_ACCESS_MAP` vaya sin mapear y **sin VA**, y que la VA del attribute CB esté
alineada a 32 MiB y no a su página—. Un `promoteEntry` que empezara en el offset 44 en
vez del 48 no se ve de ninguna otra forma.

**Y dos juegos de números que no coinciden**: la consulta se indexa por *id de
propiedad* (`NV0080_CTX_PROP_*`, 0x00…0x19) y la promoción habla de *bufferId*
(0…12). La propiedad 0x17 es el bufferId 9; usar uno por otro da un búfer del tamaño
de otro sin un solo mensaje de error. Los layouts están transcritos de
`rm/r570/nvrm/gr.h` y del `ctrl2080gpu.h` de open-gpu-kernel-modules 570.144, con
asserts de `sizeof` (560 los params, 32 la entrada, 1664 la consulta) y de `offsetof`
—el `promoteEntry` empieza en el **48**, no en el 44: el array va alineado a 8 y
`entryCount` deja cuatro bytes de relleno—.

Lo que sí quedó comprobado de paso: **no hace falta TSG ni channel group** —
`r535_chan_alloc` no crea ninguno ni pasa `hObjectBuffer`/`hContextShare`/`cid`—, así
que esa vía queda descartada como causa del `NO_MEMORY`.

**Y tres cosas de r570 que NO hay que añadir** (comprobadas en `rm/r570/gr.c`, no
supuestas): (a) el `INIT_BUG4208224_WAR` que aparece en su `gr.h` es del *scrubber* y
está detrás de un `switch (device->chipset)` con **0x162/0x164/0x166** — TU11x; GB205
es 0x1b5, así que no aplica; (b) `r570_gr_get_ctxbufs_and_zcull_info` usa **el mismo**
control y el mismo tamaño que r535 para los tamaños de contexto, sobre el subdevice, e
itera `engineContextBuffersInfo[0]` con el **índice 0 fijo** — igual que
`gsp_grctx_query(rm, 0, ...)`; (c) la consulta de zcull que añade r570 es best-effort y
es cosa del gráfico (depth culling), no del compute. La promoción va **después** del
canal y de la clase, que es el orden que sigue el bring-up.

**El `ro` del contexto ya se mapea como tal, y el PCF dejó de ser un número suelto.**
La tabla entera de `NV_MMU_VER3_PTE_PCF_*` (`nvhw/ref/gh100/dev_mmu.h`, transcrita el
2026-07-29) tiene estructura: bit 4 = ACD frente a ACE, bit 3 = NO_ATOMIC, **bit 2 =
RO**, bit 1 = PRIVILEGE, bit 0 = UNCACHED. De ahí los cuatro que usa el port, con
nombre: `0x10` VRAM RW cacheada, `0x11` sysmem RW sin cachear, `0x14`/`0x15` sus
variantes de sólo lectura. `gsp_vmm_map_flags(..., GSP_VMM_RO)` es lo que las pide;
`gsp_vmm_map` sigue siendo eso con 0. El hostcheck comprueba los cuatro PTE **con los
números a mano** (0x81/0x8d/0xa1/0xad — derivarlos con la misma expresión que el código
haría que un PCF mal elegido pasara), que el `ro` no se queda pegado en el mapeo
siguiente, y que el `UNRESTRICTED_PRIV_ACCESS_MAP` del contexto llega al PTE de sólo
lectura: el plan lo marca, el mapeo lo aplica y la traducción lo confirma.

Queda **una** desviación, dicha en `pte_encode`: upstream mapea además con `priv = 1` y
nuestro PCF es `REGULAR_*`, que es más permisivo (un acceso privilegiado a una página
regular pasa; al revés no), así que no rompe nada.

**Trampa de numeración: r535 y r570 divergen desde 0x101c.** En r535 ese código es
`NVLINK_FAULT_UP` y `0x1020` no existe; en r570 son `GSP_LOCKDOWN_NOTICE` y
`GSP_POST_NOCAT_RECORD`. Con el enum de r535 el diagnóstico apunta a NVLink en una
portátil sin NVLink. Usar siempre `rm/r570/nvrm/msgfn.h`.

**Gotcha 4 (2026-07-25): `NV_VGPU_MSG_FUNCTION_FREE` valía 27 y es 10.** El 27 es
`DMA_FILL_PTE_MEM`: liberar un objeto le pedía a GSP-RM que rellenase PTEs
interpretando los cuatro handles de `NVOS00_PARAMETERS` como descriptor. Nunca
se disparó —el único `gsp_rm_free` del árbol estaba en la rama de error de
`gsp_rm_init`, que no se tomó en HW— pero el apagado de G4 lo llama siempre.
Los otros tres números **sí** estaban bien y los confirmó el hardware
(`GSP_RM_ALLOC`=103 y `GSP_RM_CONTROL`=76 respondieron `ok`,
`GET_GSP_STATIC_INFO`=65 devolvió su propio fn). **Moraleja: que tres constantes
de una tabla estén verificadas no dice nada de la cuarta.** Verificado contra
`rm/r570/nvrm/rpcfn.h`; `UNLOADING_GUEST_DRIVER` = 47.

**Dónde mirar el `rpcfn.h`: git.kernel.org responde 403 (Anubis).** Los headers
de `linux-headers-7.0.0-*` traen el árbol de nouveau pero **solo los `Kbuild`**,
sin un `.h`. La vía que funciona es
`raw.githubusercontent.com/torvalds/linux/master/drivers/gpu/drm/nouveau/…`.

**Gotcha 5 (2026-07-25): `rpc_result` tiene DOS familias y confundirlas cuesta
caro.** Por debajo de `0xff000000` es un `NV_STATUS` normal de RM (el `0x59` de
G4a). `0xff1000xx` es `NV_VGPU_MSG_RESULT__RPC` (`rpc_headers.h` de
open-gpu-kernel-modules): **el mensaje ni llegó a RM, lo rechazó el transporte**
— el problema está en cómo lo construimos nosotros, no en lo que pedimos.
`0xff100001` = `RPC_UNKNOWN_FUNCTION`, **`0xff100002` = `RPC_INVALID_MESSAGE_FORMAT`**.
Ya están todos en `rpc_status_name` (`gsp_rpc.c`).

**G4d desbloqueado: `GET_GSP_STATIC_INFO` NO es una petición pelada.** Con
`params=NULL, size=0` GSP-RM devolvía `0xff100002` y la respuesta venía con
`len=32` (solo cabecera). Upstream la manda con
`nvkm_gsp_rpc_rd(gsp, fn, sizeof(*rpc))`, y **ese tamaño viaja en la petición**:
`r535_gsp_rpc_get` hace `rpc->length = sizeof(cabecera) + payload_size`, así que
el mensaje saliente mide 32 + 1656 = **1688 B** con el payload sin inicializar —
RM no lo lee, lo usa de **hueco donde escribir la respuesta**. El hostcheck
afirma ahora esa longitud. La cabecera de soso (`header_version`, `signature`,
`rpc_result=0xffffffff`, cálculo de `length`) es idéntica a upstream: lo único
que fallaba era el payload a cero. **Regla general para las RPC de tipo `_rd`:
manda el struct entero de ida aunque no lleve datos.**

**G4d (2/2): el espacio de direcciones, y quién construye las tablas.** La VRAM
**no la reparte RM**: nouveau se la reparte él (`r535_fb_ram_new` monta un
`nvkm_mm` sobre las regiones de `GET_GSP_STATIC_INFO` y ahí acaba la
intervención de RM) — por eso G4d 1/2 se quedaba con la lista de regiones
utilizables. En soso eso es `gsp_vram.c`: un asignador de puntero que avanza
sobre esas regiones y **no sabe liberar**, a propósito.
El espacio sí se pide a RM (`FERMI_VASPACE_A`) pero con
**`IS_EXTERNALLY_OWNED`**: las tablas las construimos nosotros y a RM solo se le
dice dónde está la raíz, con `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`. Es lo que
hace upstream para el VMM que promociona (`r535_mmu_promote_vmm`, `external=true`)
y no hay atajo por el otro lado: el camino "que las lleve RM" también exige un
directorio propio y encima copiarle sus PDE reservados. Dos desviaciones
deliberadas de upstream: las tablas van en **sysmem coherente** (sin BAR1 la CPU
no tiene ventana a la VRAM), así que el `FLAGS_APERTURE` del control es
`SYSMEM_COH` y no `VIDMEM`; y como en upstream, el vaspace cuelga de un
**cliente propio** (`NVKM_RM_CLIENT(1)`), que puede reutilizar el handle
`0xde1d0000` de device porque los handles se cuentan por cliente.

**Formato VER3 (`nvhw/ref/gh100/dev_mmu.h`, `vmmgh100.c`), seis niveles para
páginas de 4 KiB** — 57 bits de VA, y la raíz **tiene 2 entradas**, no 512
(indexa con un solo bit; el `numEntries` del control es `1 << 1`):

| nivel | bits de la VA | entradas × tamaño |
|---|---|---|
| 0 SPT | 20:12 | 512 × 8 B (PTE) |
| 1 PD0 | 28:21 | 256 × 16 B (PDE **doble**) |
| 2 PD1 | 37:29 | 512 × 8 B |
| 3 PD2 | 46:38 | 512 × 8 B |
| 4 PD3 | 55:47 | 512 × 8 B |
| 5 PD4 | 56 | 2 × 8 B ← raíz |

**Tres trampas del formato, y las tres callan.** (a) **El bit 0 de un PDE no es
"válido", es `IS_PTE`**: ponerlo "por analogía con el PTE" convierte el puntero
a la tabla de abajo en una traducción final. Lo que valida un PDE es que su
APERTURE no sea 0. (b) **APERTURE se codifica distinto en PTE y en PDE**: VRAM
es 0 en el PTE y 1 en el PDE (donde el 0 es INVALID); sysmem coherente es 2 en
los dos, que es justo lo que esconde el error si solo se prueba con tablas en
sysmem. (c) La mitad *pequeña* de la PDE doble tiene el **mismo reparto de bits**
que un PDE normal 64 bits más arriba (APERTURE 66:65, PCF 69:67, ADDRESS 115:76),
así que una sola `pde_encode` sirve para las dos.
Valores en crudo que fija el hostcheck: PTE de VRAM `…|0x81`, PTE de sysmem
`…|0x8d`, PDE de sysmem `…|0x0c`.

**Lo que G4d prueba y lo que no.** Prueba (1) que RM acepta el vaspace y el
directorio, y (2) que la traducción releída de las tablas da lo que se pidió —
`gsp_vmm_translate` recorre por direcciones escritas, no por índice de array, y
el bring-up comprueba además que **una VA sin mapear falla**. No prueba que la
GPU traduzca: eso solo lo dice el CE moviendo bytes, en G4e. `g3-check` tiene
los dos criterios separados por eso mismo.

**Gotcha 7 (2026-07-28): las clases de objeto NO se deducen del port, las dice
el chip.** El canal y el CE se pedían con `AMPERE_CHANNEL_GPFIFO_A` (0xc56f) y
`AMPERE_DMA_COPY_A` sobre una GB205, apoyándose en un comentario que afirmaba
que «GB205 comparte la ruta Ampere» — sin comprobar. `rm/gb20x.c` de nouveau
dice lo contrario: para este chip son `BLACKWELL_CHANNEL_GPFIFO_B` (0xca6f),
`BLACKWELL_DMA_COPY_B` (0xcab5) y `BLACKWELL_COMPUTE_B` (0xcec0) — la de compute
también estaba mal (teníamos la **A**, que es de GB100). Ahora
`gsp_rm_classes_probe` pide `GET_CLASSLIST_V2` (0x800292, **la V2**: la vieja
devuelve la lista por un `NvP64` del llamante, inútil por RPC) y
`gsp_rm_class_pick` elige la primera candidata que esté en el catálogo. Sin
catálogo se usa la primera y se dice en el log. La lista completa se vuelca al
serie: de ahí sale también la clase de compute de G4f.

**Y el `0x3b` no era la clase**: `INVALID_CLASS` es **0x22**. La tabla
`rm_status_hint` del port llamaba INVALID_CLASS al 0x2b (que es `INVALID_HEAP`) y
INVALID_OBJECT_PARENT al 0x2f (que es `INVALID_LOCK_STATE`); dos de siete
entradas inventadas. Verificada entera contra `nvstatuscodes.h` el 2026-07-28.
Mismo vicio que el enum de r535 apuntando a NVLink: **un nombre falso en un
mensaje de error manda el diagnóstico al lado contrario.**

**Gotcha 6 (2026-07-25): soltar la tarjeta con el GSP vivo cuelga el host.**
Una prueba VFIO congeló la máquina entera. **No fue un panic**: `efi_pstore` está
registrado en este equipo y capturó el GPF de `drm_framebuffer_cleanup` del día
24 (17 registros en `/var/lib/systemd/pstore/1784917347/`), pero de este cuelgue
no quedó **ni un registro** y el journal se corta en seco → lockup, la CPU no
llegó al handler. soso no petó: su log serie termina en el prompt. Lo que pasó es
que el `timeout 90` del script mató QEMU, y vfio-pci reseteó una GPU cuyo RISC-V
seguía ejecutando GSP-RM y haciendo DMA contra un dominio IOMMU que se estaba
desmontando. **No existía ningún camino de apagado en el port** (`grep` de
`_fini` no daba nada) y `SYS_HALT` llamaba a `qemu::exit()` en seco, así que el
`halt` tenía el mismo problema que el SIGTERM.

Lo arregla `gsp_fini.c`, réplica de `r535_gsp_fini` en su rama no-suspend:
1. `FREE` de los objetos de RM (subdevice → device → cliente),
2. `UNLOADING_GUEST_DRIVER` con los tres campos a cero,
3. esperar `MAILBOX0` del falcon GSP (`0x110040`) = `0x80000000`,
4. **quitar el bus master** (`lx_pci_clear_master`, nuevo en `pci.rs`).

El paso 4 no es de upstream y es el que de verdad protege al host: corre
**siempre**, fallen o no los tres primeros. Sin DMA da igual en qué estado
quedara el RISC-V. **No** se resetea el falcon a mano: los bits de reset del
RISC-V de GB20x no están verificados en este árbol y vfio va a resetear igual;
lo que aporta el módulo es que ese reset llegue con RM avisado y el DMA parado.
Se dispara desde `SYS_HALT` (`gpu::shutdown()`, no-op si no hay NVIDIA) y desde
userspace con el comando `GFINI` de `SYS_GPU_SUBMIT`. `l6-g1-vfio-test.sh` ya
**no** mata QEMU con `timeout`: espera el prompt, pide `halt` por SSH y solo mata
como último recurso, avisando.

**Captura de crash del host: `./scripts/l6-kdump-setup.sh`** (`--enable` /
`--disable` / `--status` / `--selftest`). Montarlo antes de gastar el siguiente
ciclo de GPU.

**Y el detalle que importa: kdump NO basta, porque kdump captura *panics* y el
cuelgue del 25 no lo fue.** Lo que lo hace capturable es
**`kernel.hardlockup_panic=1`**: el detector NMI ya estaba activo
(`nmi_watchdog=1`) pero solo avisaba; con esto una CPU atascada ≥10 s con las
interrupciones cerradas —la pinta de un reset de vfio-pci que no completa—
provoca un panic y entonces sí entra kdump. El script pone también
`panic_on_io_nmi=1` (un SERR de PCIe llega como NMI de E/S) y
`softlockup_panic=1`. Deja fuera `unknown_nmi_panic` a propósito: en portátiles
es la fuente clásica de panics espurios; actívalo a mano si hace falta.

Tras eso hay tres canales y **los tres dicen algo**: kdump (vmcore en
`/var/crash`), pstore (final del dmesg aunque kexec falle; ya funciona hoy), y
**que no aparezca nada** — eso descarta el lockup de software y apunta a nivel
máquina (machine check, error fatal de PCIe), que ningún software captura.

Notas de esta máquina: Secure Boot **desactivado** y lockdown `none`, así que
kexec no tiene pegas de firma. El `crashkernel=` va por
`/etc/default/grub.d/soso-l6-kdump.cfg` y **no** editando
`GRUB_CMDLINE_LINUX_DEFAULT`, que es donde vive el `modprobe.blacklist` del
passthrough. Precio: ~512 MiB de RAM reservados y un hard lockup ahora reinicia
en vez de quedarse colgado. **`--selftest` provoca un panic real** (`sysrq-c`):
hacerlo una vez, porque un kdump sin probar no es una red, es una suposición.

**Verificación sin GPU: `./scripts/l6-g3-gsp-hostcheck.sh`.** Compila los módulos
de los pasos 3–6 en el host con la capa lx y **un FSP simulado detrás del MMIO**
(`tools/gsp-hostcheck/main.c`) contra los blobs reales: hojas de la radix3 una a
una, tamaño del WPR meta, heap, offsets del bootloader, PTEs de la memoria
compartida, `id8` de las regiones, enlace de los boot params, imagen FMC copiada
idéntica, campos del FMC a cero, **el paquete COT byte a byte** y que un rechazo del
FSP se detecta. Cubre además el lado G4: la cadena de objetos de RM, que
`GET_GSP_STATIC_INFO` se pida con sus 1688 B y no pelada (gotcha 5), **todo G4d
2/2** (el vaspace externo con su cliente propio, el `SET_PAGE_DIRECTORY` con sus
2 entradas en sysmem, los 512 PTEs y los PDEs bit a bit contra dev_mmu.h, que
fuera del mapeo no traduzca, el reparto de VRAM y que el fini quite el
directorio antes de soltar las tablas), **el catálogo de clases** (`check_classlist`:
que se pida `GET_CLASSLIST_V2` sobre el device con sus 804 B, y que la elección
**cambie** con el catálogo — dos chips simulados, uno Blackwell y otro Ampere, que
tienen que dar clases distintas; un `pick` que devolviera siempre la primera pasaría
una prueba de un solo chip), **G4e** (canal GPFIFO + CE: `check_g4e_chan_ce`,
ALLOC, pushbuffer, fini CE→canal→VMM), y el apagado
—`FREE` con fn=10, el unload con fn=47, el handshake del mailbox y que **el bus
master se quite aunque no haya RPC vivo** (gotcha 6).
Un segundo, frente a sudo + VFIO + ~90 s del ciclo en hardware.
**Úsalo antes de gastar un ciclo de GPU** — desde el paso 6 es además la única forma
de cazar un paquete mal formado sin arriesgar un cuelgue.

Detalle y tabla de pasos 1–6 de la cadena FSP/COT en `docs/L6-G3-nvkm-scope.md`.

**Máquina del usuario (MSI Vector 16 HX AI, confirmado 2026-07-24):**
- **Híbrida**: iGPU Intel Arrow Lake (`00:02.0`, `i915`) pinta el panel; la dGPU
  NVIDIA GB205 (`01:00.0`, RTX 5070) es de render. **Pasar la dGPU a VFIO NO apaga la
  pantalla** (mi aviso genérico de "pierdes pantalla" NO aplica aquí). Pero eso **no**
  significa que la dGPU esté libre: Xorg la tiene abierta por render offload — ver el
  crash de unbind más abajo.
- La dGPU comparte grupo IOMMU con su **audio HDMI `01:00.1`**; VFIO exige ambas en
  `vfio-pci` ("group not viable" si no). `l6-g1-vfio-test.sh` bindea todas las
  funciones del slot `01:00.*` automáticamente.
- **No hay 3060** en esta máquina (solo GB205). El firmware `ga102/gsp/*` (3060) sí
  está en su linux-firmware. **Riesgo GB205**: nouveau 6.6 puede no bootear GSP
  Blackwell aunque G1 lea `NV_PMC_BOOT_0`; ruta madura = Ampere/3060.
- Estado preflight (2026-07-24, tras activar VT-d en BIOS): **DMAR presente, 30 grupos
  IOMMU**, dGPU en **grupo 10** junto a su audio HDMI (solo esas dos funciones → grupo
  viable). El kernel 7.0 activa `intel_iommu` **por defecto** al ver DMAR: no hace falta
  `l6-g1-enable-iommu.sh` ni tocar GRUB (`cmdline` sigue siendo `"quiet splash"`). Los
  checks ya no exigen `intel_iommu=on` si hay grupos > 0.
- **NO hacer unbind por sysfs de nvidia/nouveau.** PRIME está en `on-demand` (el panel
  va por la iGPU Intel), pero Xorg mantiene la dGPU abierta y `nvidia_drm` posee `fb0`:
  `echo … > /sys/bus/pci/drivers/nvidia/unbind` provocó
  `drm_WARN_ON(!list_empty(&fb->filp_head))` ×3 y un **GPF en
  `drm_framebuffer_cleanup+0x8f`** (puntero envenenado `dead000000000122`) que colgó la
  máquina — hubo reset (2026-07-24 20:22, kernel 7.0.0-28, nvidia 595.84).
  Vía segura, desde un TTY (Ctrl+Alt+F3):
  `sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm` → descarga
  `nvidia_uvm/nvidia_drm/nvidia_modeset/nvidia` (si algo usa la GPU, `modprobe -r` da
  EBUSY y no rompe nada) → luego `l6-g1-vfio-test.sh`, que ahora **rechaza** el unbind
  de drivers DRM. Vuelta atrás: `sudo ./scripts/l6-g1-vfio-restore.sh` (recarga la pila
  nvidia y recuerda arrancar `display-manager`).
- **Dos formas distintas de colgar esta máquina, no las confundas.** (a) Unbind
  por sysfs de un driver DRM vivo → GPF en `drm_framebuffer_cleanup`, **con**
  traza en pstore (2026-07-24). (b) Soltar la tarjeta con el GSP arrancado →
  lockup **sin** traza ninguna (2026-07-25, gotcha 6). La primera la previene
  `l6-g1-vfio-test.sh` rechazando el unbind; la segunda, `gsp_fini`.
- **Para iterar en G1 sin cerrar el escritorio cada vez**: bind persistente en el
  arranque con `sudo ./scripts/l6-g1-vfio-persist.sh --enable` + reboot. Escribe
  `/etc/modprobe.d/soso-l6-vfio.conf` (`options vfio-pci ids=10de:2f18,10de:2f80` +
  blacklist de la pila nvidia **y cuatro líneas `install <mod> /bin/false`**),
  `/etc/modules-load.d/soso-l6-vfio.conf` y `modprobe.blacklist=…` en GRUB.
  **Ni el `blacklist` de modprobe.d ni `modprobe.blacklist=` del cmdline frenan una
  carga por nombre o por dependencia** — solo actúan al resolver un *alias*
  (comprobado 2026-07-25: con el parámetro puesto, `nvidia` salía en `lsmod` igual).
  Quien la frena son las líneas `install`; ver el bucle de udev más abajo. Antes de tocar nada
  verifica que hay otra GPU con driver (`00:02.0 i915`) y que PRIME no está en `nvidia`.
  Deshacer: `--disable` + reboot (restaura GRUB exacto, con backup `.bak.<fecha>`).
  Mientras esté activo **no hay CUDA ni nvidia-smi en el host**. `--status` no toca nada.

## Referencias Blackwell (solo lectura)

Para GB205 **no uses `lxdde/linux/` (6.6.32) como fuente de offsets** — es anterior a
Blackwell. Vendoriza y consulta en local:

- `lxdde/reference/open-gpu-kernel-modules-570.144/` — tag **570.144**, misma versión que
  los blobs; headers `published/blackwell/gb202/*`, HAL FIFO/CE/FSP, PRI masks.
- `lxdde/reference/linux-master-nouveau/` — nouveau `master` sparse (`gb202.c`,
  `nvkm/subdev/gsp/rm/r570/`, `include/nvhw/ref/gb202/`).

Ver `lxdde/reference/README.md` y `docs/L6-reference-audit-gb205.md`. Regenerar con los
comandos del README; el árbol está en `.gitignore` (~160 MiB).

## Capa lxdde

DDE estilo `lx_emul`: compila C (drivers Linux o first-party) → `liblxdde.a`,
enlazado al kernel Rust. `xtask/src/lx_build.rs`:
- Lee `lxdde/ports/<port>/source.list`; líneas `lxdde/…` = fuentes propias, otras =
  rutas del árbol Linux 6.6.32 en `lxdde/linux/` (tarball cacheado, ya extraído).
  **Offsets y structs de GB20x: contrastar contra `lxdde/reference/`, no contra 6.6.**
- Compila con clang freestanding (`-nostdinc`, `-include autoconf.h/compat.h/kconfig.h`),
  con include dirs privados de nouveau (`drivers/gpu/drm/nouveau/{include,include/nvkm,nvkm,.}`).
- **`generate_stubs()`**: dummy `lx_emul_trace_and_stop("sym")` para cada símbolo
  undefined que ningún objeto define y que no está en `provided_symbols()`. Parsea
  `nm` distinguiendo definidos vs undefined (líneas `U nombre` de 2 campos), así que
  NO duplica símbolos definidos en objetos hermanos. `provided_symbols()` lista los
  `lx_*` que resuelve la capa Rust/C en el link final del kernel.
- Puente Rust↔C: `kernel/src/lxdde/gpu.rs` (`lx_nouveau_*`), init en
  `kernel/src/lxdde/mod.rs`, modo `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau`.

## Port nouveau (`lxdde/ports/nouveau/`)

| Archivo | Rol | Nota |
|---------|-----|------|
| `gsp_bringup.c` | Máquina de fases GSP | **hw boot** GB205: FMC→RM→CE→compute; fallback soft si no hay GPU |
| `gsp_fw.c` | Carga blobs GSP + staging GEM | valida ELF/magic; el ucode NO va a GEM |
| `gsp_rm.c` | ELF64 del ucode → `.fwimage`/firma + **radix3** verificada | fase `rm_radix3`, sin MMIO |
| `gsp_wpr.c` | Bootloader RISC-V en sysmem + **`GspFwWprMeta`** | fase `wpr_meta`, solo lee VRAM |
| `gsp_libos.c` | Colas, logs, RMARGS, **`GSP_FMC_BOOT_PARAMS`** | fases `libos_args`/`cot_ready`, sin MMIO |
| `gsp_dma.c` | `gsp_dma_buf` (equivalente de `nvkm_gsp_mem`) | reservas coherentes compartidas |
| `fsp_lx.c` | **Envío del COT** por EMEM + espera al FMC | fase `cot_sent`; escribe MMIO |
| `gsp_rpc.c` | RPCs GSP-RM + log `RC_TRIGGERED` (journal cabeza) | fase `rm_ready` |
| `gsp_rm_obj.c` | Objetos de RM (cliente/device/subdevice) + static info | fase `rm_objects` |
| `gsp_vram.c` | Reparto de VRAM sobre las regiones utilizables | puntero que avanza, sin liberar |
| `gsp_vmm.c` | **Tablas VER3 + `FERMI_VASPACE_A` externo + directorio** | fase `rm_vmm` |
| `gsp_chan.c` | Canal GPFIFO + USERD + PB; `ack_progress` (Blackwell) | COPY0 + GR0; GPFIFO 4096×8 B |
| `gsp_ce.c` | Motor CE (DMA copy) + selftest readback | **G4e GO** |
| `gsp_compute.c` | QMD v05 + PCAS 24 B + saxpy/matvec | **G4f/G5 GO** |
| `gsp_fini.c` | **Apagado ordenado** de GSP-RM + corte de DMA | fase `fini`; ver gotcha 6 |
| `fmc_lx.c` | Ruta FSP/GSP-FMC de Blackwell | valida el ELF FMC y **lee** el FSP |
| `gsp_mmio.c` | BAR0 rd32/wr32, poll, kick, **espera del GFW boot** | `kick_boot` NO arranca HW real (solo traza) |
| `gsp_chip.c` | Familia + doorbell kick (bit 30 gb20x) | |
| `gsp_top.c` | **PTOP**: motores, runlists y fault ids que publica el chip | sólo lee; sustituye a inferir índices de la tabla de RM |
| `gsp_grctx.c` | **Contexto de GR**: consulta de tamaños, plan, reserva, mapeo y `PROMOTE_CTX` | el plan es función pura y va entero en el hostcheck |
| `gsp_pramin.c` | Ventana PRAMIN → VRAM (USERD/inst) | |
| `acr_fw.c`,`falcon_lx.c`,`acr_lx.c` | ACR ola2 lx-native (AHESASC→ASB) | best-effort/soft-fail |
| `nouveau_stub.c` | pci_driver + exports C | probe vendor 0x10de |
| `nvkm_bringup_lx.c` | **puente Ola 3** | construye `nvkm_device` real + `ga102_gsp_new` |
| `nvkm_device_lx.c` | accesor `nvkm_device_subdev` | evita la tabla de chips de engine/device |
| `nvkm/subdev/gsp/{base,ga102}.c` | **nvkm real (Ola 1)** | compila+integra |
| `nvkm/core/{option,subdev,engine,memory,mm,gpuobj,firmware}.c` | **núcleo nvkm real (Ola 1 f2)** | ctors reales, no dummies |
| `nvkm/falcon/{base,fw,ga102,gm200,gp102}.c` | **falcon real (Ola 1 f2 + Ola 2)** | ctor + funcs por chip |
| `nvkm/nvfw/{fw,hs,ls,acr,flcn}.c` | **parsers firmware (Ola 2)** | `nvfw_*` |
| `nvkm/subdev/acr/{base,lsfw,tu102,ga102,ga100,gp102,gm200}.c` | **ACR real (Ola 2)** | secuencia AHESASC→ASB |
| `nvkm/subdev/{timer,mc,top,bar,instmem,fb,mmu}/{base,vmm}.c` | **subdev base real (Ola 1 f2)** | infra genérica |

**Distinción clave:** el grafo nvkm se construye en runtime; el **boot GSP y el
compute en GB205 van por la ruta lx-native** (`fmc_lx`/`gsp_*`, no por
`ga102_gsp_new` completo). G3b–G5 están en **GO en silicio** (2026-07-29). La
ruta ACR Ampere sigue escrita y sin probar (no hay 3060 en esta máquina).

**Estado (2026-07-24): 62 fuentes nvkm/lib integradas, solo 4 dummies restantes**
(`target/g3-nvkm-undefined.txt`), **todos dependientes de HW/ROM:**
`nvbios_image`/`nvbios_shadow`/`bit_entry` (lectura VBIOS por PCI ROM/MMIO) y
`nvkm_pci_msi_rearm` (necesita `struct pci_dev` real). Se resuelven integrando la capa
MMIO/PCI real (tras G1), no con más port de ficheros. `nvkm_device_subdev`/`_engine` se
proveen como accesores mínimos en `nvkm_device_lx.c` (evita la tabla de 3268 LOC de
`engine/device/base.c` que arrastraría cientos de constructores de todos los chips).
Integrado: core completo, falcon, nvfw, ACR, mmu, fb, instmem, y la **base del motor
de cómputo** (`engine/{gr,fifo,dma,falcon}` base + canales/runlist). El `engine/gr` por
chip (contexto/métodos/fw) y el compute real llegan con HW (G4, tras G1).

Primitivas ya reales en `lxdde/shim/src/shims.c`: `snprintf`/`scnprintf`/`vsnprintf`
(formateador a buffer), `alloc_page`/`page_address`/`__free_page` (página real),
`rb_*` (vía `lib/rbtree.c`), `strncasecmp`. Force-include de `<linux/types.h>` en
`compat.h` da `bool` a toda TU (p.ej. `lib/rbtree.c`); shims `export.h`/`rcupdate.h`
para código de `lib/`.

**Gotcha de build (resuelto):** `lx_build.rs` cacheaba objetos solo por mtime del
`.c`, ignorando cambios en las cabeceras del shim → objetos obsoletos. Ahora invalida
si cualquier header bajo `lxdde/shim/include/` es más nuevo (`newest_mtime`). Si dudas
del estado, `rm target/lxdde/*.o` fuerza recompilación limpia.

## Port nvkm real — workflow de integración

Roadmap del port en `docs/L6-G3-nvkm-scope.md`: Ola 1 (PCI+MMIO+mem / subdev GSP),
Ola 2 (ACR+falcon, `nvkm_ola2.list`), Ola 3 (GSP loader), Ola 4 (compute `engine/gr`).

Bucle para añadir fuentes nvkm de Linux:
1. Inventario: `./scripts/l6-g3-nvkm-inventory.sh <lista>.list` (añade temporalmente a
   `source.list`, compila, vuelca undefined a `target/g3-nvkm-undefined.txt`, restaura).
2. Errores **de cabecera** → shim mínimo en `lxdde/shim/include/` (ver abajo).
3. Cuando compila, añadir la fuente a `source.list`; símbolos nvkm sin portar quedan
   como dummy `trace_and_stop` (enlaza). Portar/puentear en olas siguientes.
4. Verificar: `cargo xtask lx-build nouveau`, `cargo xtask build`, `cargo xtask g3-check`.

### Shims de cabecera (lección Ola 1)

Las cabeceras nvkm (`nvif/os.h`) arrastran ~35 headers reales del kernel; el pesado
`<linux/slab.h>` y `<linux/pci.h>` desencadenan la avalancha
`gfp→mmzone→spinlock→preempt→thread_info→processor/cpufeature` (core arch x86 con
per_cpu/current). **No se parchea ese core**: se **sustituyen** los headers pesados por
versiones mínimas estilo lx_emul en `lxdde/shim/include/`:
- `linux/slab.h` (mapea a `lx_kmalloc/kzalloc/kfree`; actúa de prelude incluyendo
  errno/err/string/spinlock/refcount/kref/bug/notifier/scatterlist + `container_of`),
  `linux/pci.h` (forward decls opacos), `mutex/spinlock/list/bitops/firmware/io/delay/`
  `log2/vmalloc/err/string/bug/refcount/kref/notifier/scatterlist`, wrappers `asm/`
  (`errno`,`ioctl`,`mmiowb`,`bitsperlong`,`unaligned`), y stubs vacíos de subsistemas no
  usados (i2c/clk/acpi/iommu/tegra/…).
- `autoconf.h`: `CONFIG_PGTABLE_LEVELS`, `X86_*_CACHE_SHIFT`, etc.
- `linux/compiler.h`/`kconfig.h`: `barrier`, `RELOC_HIDE`, `IS_ENABLED`, atributos.

Regla: los shims son supersets mínimos; deben servir a TODOS los ports sin regresión
(verificar `cargo xtask lx-build all` + build en cada `SOSO_LXDDE_MODE`).

## Firmware GSP

`./scripts/l6-pack-firmware.sh [--repack]`: copia blobs gb205/ga102 desde
`/lib/firmware/nvidia` y **descomprime `.zst`→`.bin`** (soso no tiene zstd en kernel).
Layout: `rootfs/lib/firmware/nvidia/gb205/gsp/{bootloader,fmc,gsp}-570.144.bin` (+
`ga102/acr/{ucode_ahesasc,ucode_asb}.bin`). Luego `cargo xtask build`.

## Comandos

```bash
cargo xtask lx-build nouveau      # compila el port (incl. nvkm Ola 1)
cargo xtask g1-check              # host: IOMMU/VFIO/firmware/BAR0
cargo xtask g3-check              # bring-up GSP: firmware, módulos, fases
./scripts/l6-pack-firmware.sh     # empaqueta firmware GSP
./scripts/l6-g3-gsp-hostcheck.sh  # pasos 3-6 + G4d–G4f (chan/CE/compute) sin GPU ni sudo
./scripts/l6-g3-nvkm-inventory.sh nvkm_ola2.list   # inventario símbolos
./scripts/l6-kdump-setup.sh --status   # ¿el host capturaría el próximo cuelgue?
# Ciclo VFIO (G5): cap Gen3 → sudo ./scripts/l6-g1-vfio-test.sh
# Passthrough a mano: SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run  (apagar con halt)
```

**Nunca sueltes la tarjeta con el GSP vivo** (gotcha 6). Si lanzas el
passthrough a mano en vez de con `l6-g1-vfio-test.sh`, apaga con `halt` desde
soso —no con Ctrl-C ni matando QEMU—, que es lo que dispara `gsp_fini`. En el
log tiene que aparecer `GSP-RM apagado (… dma=off)` antes de que QEMU salga.

## Camino alternativo L6-H (CUDA en host) — **GO 2026-07-27**

`docs/L6-H-cuda-hybrid.md`: atajo práctico, **no da autonomía**. La GPU corre en el
host Linux (llama-server `-ngl`); soso se conecta vía `cuda-proxy` TCP `:11400`.

**Arranque (Docker, GPU en driver nvidia):**

```bash
docker run -d --name soso-llama --device=/dev/nvidia0 --device=/dev/nvidiactl \
  --device=/dev/nvidia-uvm --device=/dev/nvidia-modeset -p 8080:8080 \
  -v "$PWD/target:/models" ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/tinyllama-q4km.gguf -ngl 99 --host 0.0.0.0 --port 8080
cargo build -p cuda-proxy --release --target-dir target
target/release/cuda-proxy --listen 0.0.0.0:11400 --llama http://127.0.0.1:8080
```

Desde soso (QEMU): `soso-llm run tinyllama-q4km --cuda-host 10.0.2.2:11400 --prompt "hola" --max 32`

Coexiste con G1–G5 **solo si VFIO persist está off** (driver `nvidia` en el host).

## Desarrollo sin soltar la GPU (host)

| Paso | Comando |
|------|---------|
| Hostcheck | `./scripts/l6-g3-gsp-hostcheck.sh` |
| lx-build | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask lx-build nouveau` |
| build/run | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build` / `run` |
| L6-H | Docker + cuda-proxy (arriba) |

QEMU sin passthrough: `nvidia: sin GPU NVIDIA en PCI` — normal. `GSP booted (soft)` solo con GPU en PCI (VFIO).

## Riesgos

- GB205 es reciente; nouveau upstream puede ir por detrás. Fallback de validación:
  Ampere `ga102` (escrito, sin HW aquí).
- Enlace PCIe Gen5 bajo VFIO inestable en FMC (capar a Gen3; ver operativa arriba).
- No soltar la GPU con GSP vivo (gotcha 6): siempre `halt` → `gsp_fini`.
