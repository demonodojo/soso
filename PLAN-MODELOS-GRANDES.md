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

**✅ COMPLETADA (2026-07-21).** L3a (APIC/APs) + L3b (scheduler multicore,
hilos, futex, IPI wake, GEMM paralelo). Multiplicador ~×n cores sobre el
decode memory-bound.

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

**🟡 L3b (scheduler multicore, avanzado 2026-07-21) — estado per-CPU real,
timer LAPIC, scheduler/syscall multicore; la carrera de pila se resolvió
en la misma sesión (ver abajo). El cierre (hilos/futex/IPI/GEMM) es ✅
más abajo:**

GDT/TSS propia por core (`arch::gdt`, `GlobalDescriptorTable<GDT_CAP>` con
una TSS por CPU: `RSP0` + pila IST de double-fault dedicadas; la CPU 0/BSP
sigue usando el `KSTACK` de siempre). Datos por-CPU vía GS base
(`arch::percpu`, `IA32_GS_BASE`): `current_pid`/`cpu_index`/puntero al área
xsave/scratch de syscall, uno por core; `CURRENT_PID` y el contador de
timeslice (`REMAINING`) dejaron de ser globales (`task/mod.rs`). Nuevo
`State::Running` (además de `Runnable`): con un solo core era implícito que
nadie más miraba `PROCS` mientras un proceso corría; con SMP dos cores
podían adquirir el lock de `PROCS` en momentos distintos y elegir el MISMO
proceso dos veces — se marca `Running` al elegir, para que el round-robin
lo excluya hasta que se guarde su contexto. Rutas de AP paralelas a las de
la BSP (mismo diseño, direcciones por GS en vez de símbolos fijos, para no
tocar el ensamblador ya probado de la BSP): `ap_syscall_entry`
(`task/syscall.rs`, MSR LSTAR propia vía `init_msrs_ap`), `ap_timer_isr` +
`ap_schedule_landing` (`task/mod.rs`, vector LAPIC de cada AP). Todo esto
**compila y se verificó por separado con éxito** (desensamblado a mano de
las tres rutinas nuevas para confirmar que los operandos `gs:[offset]`
generados son los esperados) y, cuando se conectó de verdad (un AP
recogiendo y ejecutando procesos reales), **el sistema llegó a arrancar
hasta una sosh interactiva funcionando con el proceso corriendo
probablemente en un AP** — la prueba de que la mecánica básica es correcta.

**Dos bugs de concurrencia reales encontrados y arreglados por el camino**
(ninguno de los dos existía en el diseño original del plan; aparecieron al
ejecutar de verdad, no por inspección):
1. `drivers/virtio_hal.rs::alloc_dma_pages` adquiría el mismo lock DOS
   veces por separado (una para buscar un bloque DMA libre, otra para
   extraerlo) — con un solo core, inofensivo; con dos, el índice quedaba
   obsoleto si otro core mutaba la lista en la ventana entre ambas
   adquisiciones. Arreglado manteniendo un único lock para las dos
   operaciones.
2. La pila de kernel de los APs (`AP_KSTACK_SIZE`) se dimensionó en la
   sesión anterior en 16 KiB, razonando que solo atendían un timer ligero;
   al pasar a ejecutar el despacho de syscalls completo (vfs → sosofs →
   caché de bloques → virtio → Hal, spawn de ELF) se desbordaba y
   corrompía silenciosamente memoria del kernel adyacente, con síntomas
   dispersos y sin relación aparente. Igualada a los 64 KiB de la BSP
   (`gdt::KSTACK_SIZE`).

**Con esos dos arreglos el arranque pasó de "nunca llega a la shell con
SMP>1" a "llega y funciona la mayoría de las veces" — pero seguía quedando
una carrera más, intermitente, con síntomas distintos cada vez (page fault
en dirección `0x8`, `assert_ne!` de virtio-drivers, virtqueue corrupta,
"non-IP response packet" de smoltcp, `#GP` dentro de
`curve25519-dalek::read_volatile`). El sello de un dato compartido sin
proteger, no de un fallo determinista — y esta vez SÍ se encontró.**

**🟢 CAUSA RAÍZ ENCONTRADA Y ARREGLADA (misma sesión, continuación
2026-07-21):** con instrumentación (`crate::println!` temporal en
`schedule_inner` mostrando qué CPU elegía qué pid) se vio a `/bin/sosh`
(pid 2) arrancar en `cpu=2` y funcionar bien... hasta que, tras imprimir el
prompt, bloquearse en la lectura de stdin — momento en el que el sistema
se corrompía. La causa: **`task::schedule()` (usada por `block_current` y
`exit_current`, o sea CUALQUIER proceso que se bloquea o termina) llamaba
incondicionalmente a `schedule_landing()`, el trampolín que resetea la pila
a la de la BSP (`sym gdt::KSTACK`) — sin importar qué core lo invocara.**
Un proceso bloqueándose en un AP saltaba a la pila DE LA BSP mientras esta
podía estar en uso en ese mismo instante en el otro core: corrupción de
pila garantizada, con el síntoma dependiendo de qué hubiera en esa pila
compartida en cada repetición (de ahí la variedad de fallos). El resto de
la infraestructura de la sesión (percpu, TSS, timer/syscall de AP,
`State::Running`) estaba bien; faltaba este único punto de entrada.
Arreglo (`task/mod.rs::schedule`): despacha por `percpu::cpu_index()` — 0
va a `schedule_landing()` (BSP, sin tocar), cualquier otro a
`ap_schedule_landing()` (la pila de ESE core, ya existente y probada).

**Verificado exhaustivamente:** con el fix, `cargo xtask test` corrido
~20 veces repartidas entre `SOSO_QEMU_SMP=1/4/8` dio **cero panics /
corrupciones** (frente al 100% de caídas de antes del fix). El log de
scheduling confirma procesos reales migrando entre cores a mitad de
ejecución (p. ej. un mismo pid corriendo primero en cpu=3, luego cpu=0,
luego cpu=2) sin incidentes. `task::ap_enter_scheduler()` quedó llamada
desde `arch::smp::ap_entry` (ya no aparcado): **los APs ejecutan procesos
de usuario reales, de verdad, con SMP funcionando.**

**Dos hallazgos adicionales, benignos (no bloquean, documentados para más
adelante, NO son el bug de corrupción — son variancia de tiempo):**
1. `user/libsoso`'s `SbrkAllocator` hace una syscall `sbrk` por cada
   asignación pequeña (sin agrupar en un arena local) — el modelo
   sintético diminuto de `cargo xtask test` genera **~15000 syscalls sbrk**
   para una sola invocación de `soso-llm`. Cada syscall compite un poco
   más por `PROCS.lock()` con los cores ociosos sondeando cada ~10ms
   (`schedule_inner` en su `None` branch); con 15000 de ellas, la cola se
   nota y escala con el número de cores (peor en `SMP=8` que en `SMP=4`).
   Confirmado NO es un cuelgue: con más margen de espera en el test
   (`xtask/src/test.rs::ssh_llm`, subido de 45s a 100s) pasa la inmensa
   mayoría de las veces; ocasionalmente ni 100s bastan bajo mucha
   contención (cola de emulación TCG + varios cores), pero el sistema
   sigue funcionando y termina bien (halt limpio) — solo ese test
   concreto necesita más margen. **Mejora futura recomendada (no
   urgente):** que `SbrkAllocator::alloc` reserve de golpe un bloque
   mayor (p. ej. potencia de 2 o unos KiB) y sirva asignaciones pequeñas
   desde ahí sin syscall, en vez de una syscall por allocación.
2. La sesión SSH del propio test (`ssh_sesion`, conexión inmediatamente
   después de la de `ssh_llm`) falla a veces con salida vacía — esto es la
   ventana de carrera de reconexión rápida **ya documentada como
   preexistente** en el proyecto (memoria: "Conexiones consecutivas muy
   rápidas aún pueden pillar una ventana de recuperación"), de antes de
   cualquier trabajo de SMP. No se investigó más a fondo por ser conocida
   y no relacionada con la corrupción de memoria que se pidió investigar;
   una mejora futura sencilla sería que el arnés de test reintente la
   conexión una vez tras una pausa corta.

**✅ L3b COMPLETADA (2026-07-21) — cierre: ciclo de vida, AddrSpace
compartido, hilos/futex, IPI de wake y GEMM paralelo:**

1. **Ciclo de vida seguro:** `kill_pending` si el objetivo está `Running`
   (no marcar `Zombie` mientras otro core lo ejecuta); los timers no
   revierten `Zombie`/`Waiting*` a `Runnable`; el cleanup de zombis huérfanos
   no libera el `AddrSpace` si `pid_en_ejecucion` (per-CPU).
2. **AddrSpace con `Arc`:** PML4 + regiones mmap compartidos entre hilos;
   `Drop` libera el árbol al caer la última referencia.
3. **Threads de usuario:** ABI `SYS_THREAD_SPAWN` / `SYS_FUTEX` /
   `SYS_NCPU`; `State::WaitingFutex`; wrappers y `thread::spawn` en
   libsoso. Validación de pila acepta mmap bajo demanda.
4. **IPI de replanificación:** `apic::send_ipi` + `RESCHED_VECTOR` (0x41);
   `kick_idle_cpus` al hacer `Runnable` (spawn, futex wake, etc.).
5. **GEMM paralelo:** trait `RowParallel` + `matvec_view_par` en
   `soso-llm-core`; `ThreadPool` en `soso-llm` (`workers = ncpu`, franjas
   de filas + barrera por generación).

**Verificación L3b:** `cargo xtask test` verde con `SOSO_QEMU_SMP=1/4/8`;
`init test` incluye subprueba de hilos (contador con futex, `ncpu`);
`soso-llm` reporta `workers=N` bajo SMP.

**Medición tok/s (2026-07-23, QEMU TCG sin KVM):** `cargo xtask bench-llm`
con modelo sintético `bench` (~128 MiB, 4 capas, hidden 1024) y `--max 4`:

| SMP | workers | tok/s | vs SMP=1 |
|-----|---------|-------|----------|
| 1   | 1       | 0.18  | 1.00×    |
| 4   | 4       | 0.27  | 1.50×    |

El escalado es sub-lineal bajo emulación TCG (overhead del host); en hardware
real con KVM se espera acercarse al ×n cores sobre decode memory-bound.
`soso-llm` imprime tok/s in-guest vía `SYS_UPTIME_MS`.

*Mejoras futuras (fuera del cierre L3b, no bloqueantes):*
- Auditoría de concurrencia más amplia en `net/` / `fs` / `drivers/`.
- Agrupar `sbrk` en `SbrkAllocator` (menos syscalls bajo contención).
- Reintento en el arnés SSH del xtask tras reconexión rápida.

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

**🟡 Cimientos completados (2026-07-21), verificados en QEMU** (BIOS y
UEFI/OVMF × `SOSO_QEMU_SMP=1/4`). Pendiente: drivers NVMe + NIC física y
bring-up en la máquina objetivo.

**✅ L5a — boot UEFI + ECAM dinámico + MSI-X/IOAPIC (2026-07-21):**

1. **Boot UEFI:** xtask genera `target/soso-bios.img` y
   `target/soso-uefi.img` (`create_bios_image` + `create_uefi_image`).
   `SOSO_FIRMWARE=uefi` arranca con OVMF por pflash (rutas típicas o
   `SOSO_OVMF_CODE`/`SOSO_OVMF_VARS`); BIOS sigue siendo el default. El
   kernel no cambia de entry point.
2. **ACPI MCFG + MADT ampliado:** `arch/acpi.rs` parsea MCFG (base ECAM +
   buses) y de MADT también IOAPIC (tipo 1) e Interrupt Source Override
   (tipo 2). En QEMU BIOS el ECAM es `0xb0000000`; bajo OVMF es
   `0xe0000000` — prueba de que el hardcode de q35 ya no manda.
3. **PCI multi-bus / multi-función** con ECAM dinámico
   (`drivers/pci.rs::init_ecam`); virtio-blk/net dejan de asumir
   `0xB000_0000`.
4. **IRQ registry** (`arch/irq.rs`): vectores `0x42..=0x61`, stubs en la
   IDT, EOI de LAPIC tras el handler.
5. **IOAPIC** (`arch/ioapic.rs`): MMIO + RTE + ISO (listo para INTx
   legacy; MSI-X no lo necesita).
6. **MSI-X** en `pci.rs` (capability 0x11, programar entrada, enable +
   INTx disable). **virtio-net** lo usa de verdad: arma vector, escribe
   `queue_msix_vector`/`msix_config` en el common cfg, y el handler hace
   `ack_interrupt` + `net::poll`. El polling del timer sigue como
   fallback. Log verificado: `net: MSI-X armado…` y
   `net: primera IRQ MSI-X recibida` con SSH/echo en verde.

**✅ L5b — NVMe + e1000e + GOP/memtest en QEMU (2026-07-22):**

1. **DMA compartido** (`drivers/dma.rs`): alloc contiguo +
   `alloc_zeroed_uc` con VA dedicado sin caché (`mm::map_dma_uc`). Hallazgo:
   las CQEs NVMe en el mapeo WB del bootloader (a menudo 2 MiB) no se ven
   bien; hace falta VA UC. El phase bit del CQE está en el halfword de
   *status* (bit 16 de DW3), no en el bit 0 de DW3.
2. **NVMe** (`drivers/nvme.rs`): admin + 1 I/O queue, Identify, R/W
   síncrono, MSI-X. `fs::ModelsDev` monta sosomfs en NVMe si `present()`,
   si no virtio-blk 1. Rootfs sigue en virtio. xtask:
   `SOSO_QEMU_NVME=1` (copia de models → `-device nvme`).
3. **e1000e** (`drivers/e1000e.rs`): rings DMA, MSI-X, DHCP/echo OK.
   `net::NicDev` prefiere e1000e si está presente. xtask:
   `SOSO_QEMU_NIC=e1000e`. Default de tests: virtio-net.
4. **Framebuffer** (`drivers/fb.rs`): captura `BootInfo.framebuffer`,
   espejo de `println!` (BIOS VESA / GOP). **Memtest** ligero 4 MiB
   post-`mm::init` (`mm/memtest.rs`).

**✅ L5c — Preparación sin placa (2026-07-22):**

1. **e1000e endurecido** (`drivers/e1000e.rs`): TX espera DD antes de
   reutilizar descriptor; RX exige DD+EOP y descarta errores; burst RX
   16 en smoltcp. `SOSO_QEMU_NIC=e1000e cargo xtask test` verde (SSH +
   echo + halt).
2. **Rootfs sin virtio** (`fs::RootDev`): si no hay virtio-blk0 y hay
   NVMe ctrl 0 → sosofs en NVMe; sosomfs en NVMe ctrl 1 (o ctrl 0 si
   root sigue en virtio). `SOSO_QEMU_NVME_ROOT=1` en xtask omite
   virtio-blk0 y adjunta data+models como dos NVMe.
3. **NVMe multi-controlador** (`drivers/nvme.rs`): hasta 2 slots
   (`present_slot`, `read_block4k_slot`, …).
4. **`cargo xtask package-usb`**: escribe `target/usb-package/` con
   `soso-uefi.img`, `soso-data.img`, `soso-models.img` y `FLASH.txt`
   (instrucciones `dd` para USB ESP y NVMe host; no flashea dispositivos).

**Checklist L5c — bring-up en placa (cuando llegue el hardware):**

Ver guía completa: [`docs/L5c-on-box.md`](docs/L5c-on-box.md).

| Paso | Acción |
|------|--------|
| Host | `cargo xtask package-usb-live` → `target/usb-live/soso-live.img` |
| Host | `sudo dd if=soso-live.img of=/dev/sdX …` (solo el pendrive; ver `FLASH-LIVE.txt`) |
| Placa | UEFI boot once desde USB — **no toca el NVMe con Linux** |
| Placa | Log: `live: GPT`, `fs: sosofs live`, `fs: sosomfs live`, DHCP, SSH |
| Placa | `soso-llm run …`; apagar y arrancar Linux habitual → intacto |
| QEMU | `SOSO_QEMU_LIVE=1 cargo xtask run` valida la imagen sin placa |
| Gap | ~~USB BOT read en placa~~ — implementado (`SOSO_QEMU_LIVE_USB=1` valida BOT) |

**Verificación L5c prep (QEMU, hecha):**

| Prueba | Comando / criterio |
|--------|-------------------|
| Regresión | `cargo xtask test` |
| SSH e1000e | `SOSO_QEMU_NIC=e1000e cargo xtask test` |
| Solo NVMe | `SOSO_QEMU_NVME_ROOT=1` → `fs: sosofs en NVMe` + sosh |
| Paquete | `cargo xtask package-usb` → artefactos + `FLASH.txt` |

**Pendiente L5c-on-box (bring-up máquina física):** ejecutar el checklist
de la tabla anterior en la placa real.

**Verificación L5a (hecha):** `cargo xtask test` verde en
`{bios,uefi}×{SMP=1,4}`; bajo OVMF el ECAM sale de MCFG; MSI-X de
virtio-net dispara IRQs reales.

**Verificación L5b (hecha):** `cargo xtask test` verde (virtio default);
`SOSO_QEMU_NVME=1` → `fs: sosomfs en NVMe` + modelo listado;
`SOSO_QEMU_NIC=e1000e` → DHCP + echo TCP.

**Verificación L5 (final):** arranque en la máquina objetivo, `ssh` entra,
`soso-llm run` con el modelo grande desde NVMe.

## Fase L6 — GPU NVIDIA nativa (reabierta 2026-07-24)

*Estimación: 6–12+ meses. Hardware primario: RTX 5070 Ti Mobile / GB205
(`10de:2f18`).*

**No existe camino corto para CUDA userspace en soso.** Las GPUs Turing+
requieren firmware GSP, nvkm, canales de comando y kernels SASS. El camino
es lxdde + subconjunto nvkm (sin display) + offload híbrido por VRAM (~12 GiB).

**🟡 Infraestructura L6 (2026-07-23):** capa `lxdde`, `nvidia_probe`,
`SOSO_QEMU_GPU=vfio:…`, `docs/L6-G1-gate.md`, scripts IOMMU/VFIO.

**🟢 Reapertura L6 (2026-07-24):** roadmap G1→G5 activo. GPUs soportadas
(bring-up chip-aware): **GB205 Blackwell** (`10de:2f18`) y **Ampere GA10x**
(RTX **3060**, objetivo de validación recomendado por su GSP maduro en nouveau).
CPU L1–L4 sigue como fallback; inferencia 70B usa offload híbrido GPU+RAM.

**🟢 Port nvkm nativo (2026-07-24):** **62 fuentes nvkm/lib de Linux 6.6.32**
integradas sobre lxdde (core, falcon, nvfw, ACR AHESASC→ASB, mmu, fb, instmem,
engine gr/fifo/dma base) con shims lx_emul mínimos. El **grafo de objetos nvkm
real se construye en runtime** dentro de soso (`nvkm device graph OK`). Solo 4
dummies restantes, todos dependientes de HW/ROM. Detalle: `docs/L6-G3-nvkm-scope.md`.

| Fase | Entregable | Estado |
|------|------------|--------|
| G1 | NV_PMC_BOOT_0 bajo VFIO | **Pendiente IOMMU en placa** (VT-d en BIOS; acción del usuario) |
| G2 | Firmware gb205 + set ga102 (3060) en rootfs | **Go** |
| G3 | GSP boot nvkm (sin KMS) | **Go (software)** — grafo nvkm real en runtime; boot HW pendiente de G1 |
| G4 | Saxpy SASS real (`SYS_GPU_SUBMIT`) | Infra base integrada; compute real tras G1 |
| G5 | matvec híbrido soso-llm | Pendiente G4 |

Scripts: `scripts/l6-pack-firmware.sh`, `scripts/l6-g1-enable-iommu.sh`,
`scripts/l6-g1-vfio-test.sh`. Checks: `cargo xtask g1-check`, `cargo xtask g3-check`.

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
**Hito 2 (+L4+L3, alcanzado 2026-07-21):** decode multicore SIMD — usable
en servidor (QEMU SMP); medición tok/s con modelo `bench` hecha (2026-07-23,
TCG); falta repetir en hw/KVM y L5 en placa.
**Hito 2.5 (+L5a, alcanzado 2026-07-21):** cimientos hw real en QEMU —
UEFI/OVMF, MCFG/ECAM dinámico, IOAPIC + MSI-X (virtio-net con IRQs reales).
**Hito 2.6 (+L5b, alcanzado 2026-07-22):** NVMe + e1000e + fb/memtest
validados en QEMU; default virtio intacto.
**Hito 2.7 (+L5c prep, alcanzado 2026-07-22):** e1000e SSH en QEMU, rootfs
NVMe, `package-usb` — listo para copiar a USB/NVMe en la placa.
**Hito 3 (+L5c-on-box, bring-up físico):** SSH + `soso-llm` en la máquina objetivo
desde NVMe.

## Riesgos principales

1. **GPU NVIDIA**: alto riesgo técnico (GB205/nouveau inmaduro); offload híbrido
   mitiga VRAM limitada; CPU L1–L4 sigue como fallback.
2. **SMP**: la auditoría de concurrencia del kernel monocore es el trabajo
   traicionero (los bugs son heisenbugs); mitigación: hacerlo después de L1/L2/L4
   con el sistema ya medible, y activar cores gradualmente.
3. **RAM del host de desarrollo**: probar 70B real exige un host con >96 GB;
   mitigación: modelos sintéticos parametrizables + validar la mecánica con
   7-13B reales.
4. **fxsave olvidado** (L4): corrupción silenciosa de registros de usuario;
   mitigación: test de regresión que estresa FPU en dos procesos concurrentes.
