# Plan: modelos grandes (70B+) en soso sobre hardware real

**Objetivo:** ejecutar modelos llama de 70B+ parámetros (~40 GB en Q4_K, ~70 GB
en Q8_0) en máquinas físicas multicore. La máquina objetivo tiene GPU NVIDIA.

**Fecha:** 2026-07-15 · **Estado de partida:** pipeline LLM funcional en QEMU
(modelo tiny, mmap con paginación bajo demanda, runtime llama
RoPE/GQA/SwiGLU/Q8_0, decode greedy monocore escalar).

---

## Diagnóstico: qué limita hoy el tamaño y la velocidad

| # | Límite | Dónde | Impacto en 70B |
|---|--------|-------|----------------|
| 1 | Ventana mmap de usuario ~1 GiB | `soso-abi` (`MMAP_BASE=0x2000_0000`, `MMAP_LIMIT=0x5f00_0000`) | No caben ni los shards de un 7B |
| 2 | `load_f32` **copia** los pesos en cada uso | `soso-llm-core/src/source.rs` + `LayerScratch.w` | 40-70 GB de memcpy **por token** — inviable |
| 3 | Page faults de 4 KiB, uno a uno, con memcpy vía caché de bloques | `kernel/task/mod.rs::handle_mmap_fault` → `sosomfs::read_range` | Warmup de un 70B: ~10M faults → horas |
| 4 | `allocate_frame` es O(n) sobre el mapa de memoria | `kernel/mm/frame.rs` (`usable_frames().nth(next)`) | Cuadrático: 25M frames para 100 GB — inutilizable |
| 5 | Monocore: PIC 8259 + PIT, sin APIC, sin APs, sin threads | `kernel/arch/`, `kernel/task/` | Un solo core de decenas |
| 6 | Userspace **sin SIMD** (`x86_64-unknown-none` = soft-float) y kernel sin fxsave/xrstor | `user/.cargo/config.toml`, `task/mod.rs` (context switch solo GP regs) | GEMM escalar puro; habilitar SIMD requiere guardar estado FPU en el kernel |
| 7 | Sin Q4_K | `convert-gguf`, `quant.rs` | 70B solo cabría en Q8 (70 GB); Q4_K_M (~40 GB) es el formato práctico |
| 8 | Boot BIOS + solo drivers virtio | `xtask`, `kernel/drivers/` | No arranca en una máquina física |
| 9 | Sin backend GPU | — | Ver Fase L6: en NVIDIA es el punto crítico |

**Números de referencia (70B, decode memory-bound, ~1 pasada de pesos/token):**

| Máquina | Ancho de banda RAM | Q4_K (~40 GB) | Q8_0 (~70 GB) |
|---|---|---|---|
| Desktop DDR5 2 canales (96 GB) | ~80 GB/s | ~1.5-2 tok/s | no cabe holgado |
| Workstation/servidor 8-12 canales (256 GB+) | ~300-460 GB/s | ~6-10 tok/s | ~4-6 tok/s |

Estos son los techos teóricos con SMP+SIMD bien hechos (nivel llama.cpp). Sin
GPU no hay más; con un solo core escalar se queda en ~0.05 tok/s.

---

## Fase L1 — Memoria grande y zero-copy (la base de todo)

**✅ COMPLETADA (2026-07-15).** Verificado: `cargo xtask test` en verde y un
modelo sintético de 1.34 GiB (10 capas, hidden 2048, GQA 32/8, vocab 32k)
cargado y ejecutado end-to-end por SSH en QEMU `-m 6G` en ~114 s bajo TCG
(sin KVM en el host de desarrollo; en hardware real esto es órdenes de
magnitud más rápido). Notas de implementación: la ventana mmap quedó en
[64 GiB, 480 GiB) y `USER_MAX` subió a 512 GiB; `AddrSpace::free` y
`unmap_range` entienden páginas de 2 MiB; el builder de sosomfs escribe en
streaming (nunca carga un fichero entero en RAM); `soso-llm` acepta
`--max <n>`. Ojo con los tests por SSH: si el cliente cierra stdin, la
sesión deja de entregar la salida del servidor — mantener stdin abierto
como hace `ssh_llm`.

*Estimación: 1-2 semanas. Todo verificable en QEMU con `-m` grande.*

1. **xtask parametrizable:** `SOSO_QEMU_MEM`/`SOSO_QEMU_SMP` (o flags `--mem`,
   `--smp`) para `run`/`test`; default 2G/1 para no romper el E2E actual.
2. **Layout de VA de usuario:** el usuario ya posee todo el L4[0] (0..512 GiB).
   Mover la región mmap a `[0x10_0000_0000, 0x1F0_0000_0000)` (~448 GiB de
   ventana) en `soso-abi` y `task/mmap.rs`; subir `BRK_MAX`. ELF y pila no se
   tocan.
3. **Frame allocator O(1):** sustituir el `nth()` lineal por un cursor por
   región (región actual + offset) y free-list; añadir asignación de bloques
   de 2 MiB alineados (512 frames contiguos) para huge pages.
4. **Huge pages en el fault de mmap:** si la región es de un shard sosomfs
   alineado (el builder ya alinea shards ≥2 MiB con `ALIGN_HUGE_2M`), servir
   faults de 2 MiB: frame de 2 MiB + lectura de 2 MiB de disco de una vez +
   `map_to` con `PageSize::Size2MiB`. Reduce 10M faults a ~20k para 40 GB.
5. **Lectura directa a frame (bypass de caché):** para shards `CACHE_STREAM`,
   `read_range` debe poder leer del virtio-blk directamente al frame destino
   (DMA al buffer físico) sin pasar por `BlockCache` ni memcpy intermedio. La
   caché queda para manifest/index/metadatos.
6. **Zero-copy en el runtime:** nuevo método en `TensorSource`:
   `map_tensor(name) -> &[u8]` (vista sobre el mmap, sin copia). El payload de
   los shards debe quedar alineado a 64 B → padding de la cabecera `.som` a
   `CACHE_ALIGN` en `pack_shard` (cambio de formato menor, regenerar modelos).
   `matvec_f32`/kernels cuantizados leen directamente de la vista; desaparecen
   `LayerScratch.w` y las copias por token. Los pesos residen una sola vez en
   los frames del page cache de mmap.
7. **KV cache f16:** a 70B con 4k de contexto el KV f32 son ~2.6 GB y crece
   con `max_seq`; guardar K/V en f16 (convertir al vuelo) lo deja en ~1.3 GB.
8. **mkmodel-soso parametrizable** (`--layers --hidden --ffn --vocab`) para
   generar modelos sintéticos de decenas de GB y medir sin descargar nada.

**Verificación L1:** modelo sintético de 30-60 GB en QEMU con `-m 96G`
(según RAM del host); medir warmup (< minutos) y tok/s; `cargo xtask test`
sigue en verde con el default 2G.

## Fase L2 — Q4_K y UX de inferencia

**✅ COMPLETADA (2026-07-15).** Verificado end-to-end con un modelo real:
**TinyLlama-1.1B-Chat Q4_K_M genera texto coherente dentro de soso** ("Once
upon a time → , there was a young woman"), streaming token a token por SSH.
En host nativo (example `hostrun` de soso-llm-core, el harness rápido de
calidad): "The capital of France is → Paris." a ~2.5 tok/s escalares.
Hallazgos de la fase:
- **Faltaba la proyección Wo (`attn_output`)** en el runtime y el conversor —
  el modelo emitía logits con forma plausible pero contenido basura; se cazó
  con el hostrun comparando la predicción tras solo BOS (RoPE identidad).
- El quirk de alineación de `x86-interrupt` con código de error afectaba a
  TODOS los caminos profundos de los handlers, no solo a mmap: `kill_current`
  desde un page fault de usuario hacía GPF en `close_all_fds`, y el propio
  formateo del panic se truncaba. Generalizado en `con_rsp_alineado`
  (trampolín naked) para mmap, kill y el panic handler.
- `Q6_K` (típico en `output.weight` de los Q4_K_M) se convierte a Q8_0.
- sosh no interpreta comillas: `--prompt` toma palabras hasta el siguiente
  flag.

*Estimación: ~1 semana.*

1. **Q4_K end-to-end:** convert-gguf pasa los super-bloques Q4_K tal cual
   (256 elems, escalas 6-bit); `quant.rs` implementa dequant + **matvec
   fusionado** (descuantizar en registros dentro del bucle, no a un buffer
   f32). Es el formato en el que existen los 70B descargables.
2. **Sampling:** temperature/top-k/top-p además de greedy (`--temp`, `--top-p`).
3. **Streaming de tokens:** imprimir token a token (a ~1-5 tok/s es la
   diferencia entre usable e inútil); flush del canal SSH por token.
4. **Prueba con GGUF real pequeño** (TinyLlama/Qwen 1-3B Q4_K) end-to-end.

## Fase L3 — SMP

*Estimación: 3-6 semanas. Es el multiplicador grande (~×n cores).*

**🟡 L3a completada (2026-07-21):** ACPI (RSDP→RSDT/XSDT→MADT) + LAPIC
(x2APIC por MSR o xAPIC clásico por MMIO, detectado por CPUID) + arranque de
APs por INIT-SIPI-SIPI con trampolín real→protegido→largo copiado a una
página física reservada (`arch::acpi`, `arch::apic`, `arch::smp`). Verificado
con `cargo xtask test` en verde con `SOSO_QEMU_SMP=4` y `SOSO_QEMU_SMP=8`
(los 8 APs llegan a modo largo y se anuncian: `smp: 8/8 CPUs en línea`) sin
regresión en el caso por defecto (`SOSO_QEMU_SMP=1`).

- **Hallazgo (bug real, no solo pendiente de escribir):** el código de
  arranque de APs ya estaba escrito pero nunca se había ejecutado — con
  `SOSO_QEMU_SMP=4` el arranque se colgaba y QEMU salía con código 0 antes
  de llegar a la shell. El log serie mostraba los marcadores `AB` (modo real
  → protegido) pero nunca `C` (modo largo): los APs hacían **triple fault**
  justo al activar `CR3`+paginación, porque en ese instante su `RIP` seguía
  en la página física baja del trampolín (`0x8000`) y el mapeo del kernel
  solo cubre la RAM vía `phys_to_virt` (con offset), sin una entrada de
  identidad VA==PA en esa dirección baja. Arreglado con
  `mm::ensure_identity_mapped` (mapeo VA==PA bajo demanda, análogo a
  `ensure_mmio_mapped`) llamado en `arch::smp::init` antes de lanzar el
  primer AP. Moraleja para L3b: nada de lo ya escrito en este área estaba
  realmente probado — cada pieza nueva necesita un `cargo xtask test` con
  `SOSO_QEMU_SMP>1` antes de darla por buena.

**🟡 L3b (parcial, 2026-07-21) — estado per-CPU real y timer LAPIC:**
GDT/TSS propia por core (`arch::gdt`, `GlobalDescriptorTable<GDT_CAP>` con
una TSS por CPU: `RSP0` + pila IST de double-fault dedicadas; la CPU 0/BSP
sigue usando el `KSTACK` de siempre para no tocar el asm desnudo del
scheduler/syscall existente). Cada AP, tras anunciarse, ejecuta
`gdt::init_cpu(cpu)` (índice parcheado en la página del trampolín,
`CPUIDX_OFF`), calibra su LAPIC con `apic::timer_periodico` y hace `sti`; el
vector (`apic::TIMER_VECTOR`) tiene un handler mínimo (solo EOI) en la IDT
compartida. Verificado con `cargo xtask test` completo (arranque + LLM +
SSH + apagado, minuto largo de duración) en `SOSO_QEMU_SMP=1/4/8`: cada AP
recibe su timer periódico sin fallos ni interferencia entre cores.
- **Hallazgo:** al dimensionar la GDT para N TSS (`GlobalDescriptorTable<GDT_CAP>`,
  descriptores de sistema de 2 slots) hay que contar también el descriptor
  nulo implícito del slot 0 (`GDT_CAP = 1 + planos + 2×MAX_CPUS`) — olvidarlo
  da `"GDT requires two free spaces to hold a SystemSegment"` en tiempo de
  arranque, no en compilación (el build limpio no lo detecta).
- Este timer de los APs todavía **no hace nada útil** (solo EOI): es
  scaffolding para el paso siguiente, no scheduling real.

*Restante (L3b), estimación 3-5 semanas:*

1. **Per-CPU vía GS base:** `CURRENT_PID` (`task/mod.rs`) y `TIMER_FPU`
   (`arch/fpu.rs`, hoy `static mut` "porque soso es monocore") de global a
   array por-CPU. Más delicado: el scheduler y la entrada de syscall
   (`task/mod.rs`, `task/syscall.rs`) referencian `KSTACK` por símbolo desde
   ensamblador desnudo (`sym gdt::KSTACK`) — llevar esto a otros cores exige
   leer la pila/estado del core actual desde ese mismo ensamblador (típico:
   `IA32_GS_BASE` apuntando a una struct `PerCpu` propia de cada core,
   `mov rax, gs:0`) antes de poder generalizar `schedule_inner`/
   `syscall_entry` más allá de la BSP.
2. **Auditoría de concurrencia:** hoy los spinlocks (`PROCS`, `FRAME_ALLOC`,
   caches de FS, `net::poll`, colas SSH) asumían que un core no se
   interrumpe a sí mismo; con varios cores ejecutando de verdad hay que
   revisar cada sección crítica y las suposiciones "el kernel no se
   preempta".
3. **Scheduler multicore:** con (1) resuelto, cada AP sustituye su
   idle-loop por una variante de `schedule_inner` que compite por el mismo
   lock global de `PROCS` (suficiente para pocas decenas de cores) + IPIs
   (`apic::icr`, ya usado para SIPI) para despertar cores ociosos.
4. **Threads de usuario mínimos:** syscall `thread_spawn` (mismo PML4, pila
   nueva) + futex-lite (`wait`/`wake` sobre una dirección) para barreras.
5. **GEMM paralelo:** repartir filas del matvec entre N worker threads con
   barrera por token. El decode es memory-bound: escala bien hasta saturar
   canales de memoria.

**Verificación L3b:** `init test` + suite nueva de threads; tok/s escala
~lineal hasta 4-8 cores con el modelo sintético grande.

## Fase L4 — SIMD

**✅ COMPLETADA (2026-07-15).** Userspace con target propio
(`user/x86_64-soso-user.json`: SSE..AVX2+FMA, build-std) y kernels
vectorizados (`gemm.rs::avx2`): matvec f32 (FMA, 2 acumuladores), Q8_0 y
Q4_K fusionados (contribución `x·(d·sc·q − dmin·m)` sin sumas horizontales
intermedias). TinyLlama Q4_K_M en host: 2.65 → **6.05 tok/s** (~2.3×), mismo
texto exacto. Hallazgo importante: **fxsave NO basta para el cambio de
contexto** — solo preserva XMM; con dos procesos AVX alternándose cada uno
acaba con las mitades altas YMM del otro. Se usa **xsave64/xrstor64**
(máscara x87|SSE|AVX, área de 1024 B alineada a 64) en `arch/fpu.rs`:
`timer_isr` guarda antes de `net::poll` y `schedule_inner` restaura al
reanudar; el page fault handler preserva alrededor de `handle_mmap_fault`;
las syscalls no preservan (ABI: vectoriales caller-saved, los wrappers de
libsoso llevan `clobber_abi("C")`). `sse::enable` habilita CR4.OSXSAVE +
XCR0(x87|SSE|AVX) y exige XSAVE. El test `init test` incluye ahora dos hijos
"fpu" concurrentes que verifican la integridad YMM bajo desalojos (cazó el
bug de fxsave a la primera). También se corrigió de paso la última página
parcial de un mmap (lectura recortada a `file_len`, resto a cero) — rompía
`mmap /etc/motd` desde L1 sin que la suite lo delatara.

*Estimación: 1-2 semanas. Puede adelantarse a L3 (es independiente).*

1. **Estado FPU por proceso en el kernel:** área `fxsave64` (o `xsave`) en
   `Process`, guardar/restaurar en el context switch y en el camino
   syscall/interrupt. **Requisito previo estricto**: hoy el kernel usa SSE y
   el usuario no; en cuanto el usuario use XMM/YMM sin esto, corrupción
   silenciosa.
2. **Target de userspace con SIMD:** JSON propio (como el del kernel) con
   `+sse2,+avx2,+fma` (y detección CPUID en runtime para elegir kernel).
3. **Kernels vectorizados:** matvec f32/Q8_0/Q4_K con intrinsics AVX2 (FMA,
   `_mm256_maddubs`/estilo llama.cpp para cuantizados); rmsnorm/softmax
   vectorizados son secundarios (el matvec es >95% del tiempo).

## Fase L5 — Hardware real (headless por SSH)

*Estimación: 4-8 semanas, muy dependiente de la placa concreta.*

1. **Boot UEFI:** el crate `bootloader` ya genera imagen UEFI
   (`create_uefi_image`); xtask produce ambas. ACPI completo (MADT ya de L3,
   MCFG para ECAM real).
2. **NVMe:** driver propio (admin queue + I/O queues, MSI-X). Es el disco de
   modelos real; sosomfs no cambia (habla `BlockDevice`).
3. **NIC física:** un driver concreto según la máquina (Intel igb/e1000e es
   lo más documentado); smoltcp no cambia.
4. **Consola:** serie si la placa la tiene; si no, framebuffer UEFI GOP para
   diagnóstico de arranque y todo lo demás por SSH.
5. **Realidad a asumir:** sin USB no hay teclado local (fuera de alcance);
   máquina headless administrada por red. Memtest propio ligero al arrancar.

**Verificación L5:** arranque en la máquina objetivo, `ssh` entra,
`soso-llm run` con el modelo grande desde NVMe.

## Fase L6 — GPU NVIDIA: spike de investigación con go/no-go

*Estimación del spike: 2 semanas. La implementación, si es "go": 6-12+ meses.*

Ser claros: **no existe camino corto para cómputo NVIDIA en un kernel
propio.** Las GPUs modernas (Turing+) requieren cargar el firmware GSP
propietario, negociar con él la inicialización (lo que hace nouveau en
Linux), construir canales de comando, gestionar VRAM/page tables de GPU, y
generar código SASS para los kernels (no hay compilador utilizable fuera del
stack de NVIDIA/mesa-NAK). Cada pieza es un proyecto.

**Spike (2 semanas):** sobre la GPU concreta de la máquina objetivo,
evaluar: (a) qué generación es y si GSP/OpenRM documenta lo mínimo, (b)
cuánto de nouveau/NVK es portable a un kernel no-Linux, (c) si un kernel
SASS precompilado (un saxpy) puede lanzarse con inicialización mínima.
Con eso, decisión:

- **Go:** roadmap GPU propio (6-12+ meses, alto riesgo).
- **No-go (probable):** el motor de 70B en soso es CPU SMP+SIMD (fases
  L1-L4), que en un servidor multicanal da 4-10 tok/s reales. Si la GPU es
  irrenunciable, las alternativas honestas son cambiar el hardware objetivo
  a Intel/AMD (ISA y firmware documentados — sigue siendo un proyecto de
  meses) o replantear el nicho de soso.

---

## Orden de ejecución y dependencias

```
L1 (memoria+zero-copy) ──► L2 (Q4_K+UX) ──► L4 (SIMD) ──► L3 (SMP) ──► L5 (hw real) ──► L6 (spike GPU)
```

L4 antes que L3 es deliberado: SIMD es más barato, es prerequisito parcial
(fxsave) y multiplica lo que ya hay; SMP multiplica después sobre kernels ya
vectorizados. L6 puede lanzarse en paralelo en cualquier momento como
investigación.

**Hito 1 (L1+L2, ~3 semanas):** un 70B Q4_K genera texto en QEMU con RAM
grande, a velocidad de un core vectorizable.
**Hito 2 (+L4+L3, ~2 meses):** decode multicore SIMD — usable en servidor.
**Hito 3 (+L5, ~3-4 meses):** todo lo anterior en la máquina física por SSH.

## Riesgos principales

1. **GPU NVIDIA**: riesgo alto de no-go; el plan no depende de ella para 70B.
2. **SMP**: la auditoría de concurrencia del kernel monocore es el trabajo
   traicionero (los bugs son heisenbugs); mitigación: hacerlo después de L1/L2/L4
   con el sistema ya medible, y activar cores gradualmente.
3. **RAM del host de desarrollo**: probar 70B real exige un host con >96 GB;
   mitigación: modelos sintéticos parametrizables + validar la mecánica con
   7-13B reales.
4. **fxsave olvidado** (L4): corrupción silenciosa de registros de usuario;
   mitigación: test de regresión que estresa FPU en dos procesos concurrentes.
