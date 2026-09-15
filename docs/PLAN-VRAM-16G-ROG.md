# Plan para aprovechar los 16 GiB de VRAM de la ROG

Fecha: 2026-09-14. Estado: implementado en host (2026-09-14); validación en placa pendiente.

## Conclusión

La ROG de los logs tiene una **RTX 3080 Laptop, GA104, con 16 GiB de VRAM
dedicada**. La explicación «RTX 3050 / GA107 de 4 GiB y el resto compartida»
es incorrecta para este equipo. El código y la documentación del proyecto
contienen esa identificación equivocada y hasta hay un test que la exige.

El límite de capacidad confirmado es la **ventana virtual G6 de 8 GiB**.
Ampliarla permite acceder al resto del framebuffer utilizable. El objetivo es
usar el pool que deja RM, aproximadamente 15,7 GiB antes de otras reservas del
driver, respetando firmware y contextos; no entregar los 16 GiB físicos íntegros
a los pesos.

Mistral merece un diagnóstico separado: el modelo convertido disponible en
`target/mistral-7b-model` cabe por tamaño incluso en la ventana actual de 8 GiB.
Eliminar ese techo no demuestra por sí solo que la inferencia quede residente
ni que desaparezca su lentitud.

## Evidencia y origen del error

Fuente de placa: `target/usb-diagnostic-2026-09-13-run9/SOSOLOG.txt`.
Es un arranque anterior, no una prueba nueva realizada durante esta revisión.

| Línea | Evidencia |
|---|---|
| 3 | `heap 512 MiB de 64874 MiB utilizables`: RAM del sistema, independiente del framebuffer |
| 268 | `boot0=0xb74000a1 dev=0x249c familia=ga107 vram=16384MiB` |
| 298 | `Ampere WPR layout FB=16384 MiB` |
| 443 | `GSP static info: 'NVIDIA GeForce RTX 3080 Laptop GPU' VRAM=16384 MiB utilizable=16070 MiB` |
| 456 | `VRAM repartible: 16070 MiB en 1 región(es)` |
| 504 | `techo residente ~8192 MiB de 16068 MiB de pool; ventana VA 8192 MiB, tablas libres 87` |
| 626 | `GSP booted (hw, booter_load Ampere + RPC, 16384 MiB VRAM)` |
| 900–907 | `backend GPU` y generación terminada; sin desglose de residencia ni tok/s |

El log G6 se emite antes de completar las reservas de compute/GR: **16068 MiB
no es una promesa del presupuesto final de `ask`**. Hay que medirlo después del
bring-up completo.

Confirmación independiente:

- NVIDIA identifica `249C` como RTX 3080 Laptop en su
  [tabla de GPUs soportadas](https://download.nvidia.com/XFree86/Linux-x86_64/470.182.03/README/supportedchips.html).
  La referencia local `lxdde/reference/open-gpu-kernel-modules-570.144/README.md:781`
  también lo recoge. El nombre de RM desambigua variantes/subsistemas.
- `(0xb74000a1 >> 20) & 0x1ff = 0x174`. Linux 6.6.32 local asigna ese chipset a
  `nv174_chipset`, cuyo nombre es `GA104` en
  `lxdde/linux/drivers/gpu/drm/nouveau/nvkm/engine/device/base.c:2657`.
- [gsp_chip.h](../lxdde/ports/nouveau/gsp_chip.h) define erróneamente
  `GA107_DEVICE_ID = 0x249c` con comentario «RTX 3050 Mobile»;
  [gsp_chip.c](../lxdde/ports/nouveau/gsp_chip.c) propaga el nombre.
- [gsp_bringup.c](../lxdde/ports/nouveau/gsp_bringup.c), `vram_for_device`,
  asigna 4 GiB a ese ID **sólo como fallback**. El arranque lee primero
  `0x1183a4` mediante `gsp_wpr_vidmem_size()` y aquí obtiene 16 GiB.
- [gpu.rs](../kernel/src/drivers/gpu.rs) publica además un nombre NVIDIA
  fijo de GB205: tampoco debe usarse para identificar esta placa.

BAR1 también mide 16 GiB en el log, pero una apertura PCI no demuestra por sí
sola la capacidad física. La prueba es la concordancia registro → WPR → RM →
regiones FB. Los avisos de recorrido BAR1 son otro asunto: CE ya copia a VRAM
y pasa su selftest, por lo que arreglar BAR1 no es requisito para ampliar G6.

## Límites que hay que resolver

1. **VA fija:** [gsp_buf.h](../lxdde/ports/nouveau/gsp_buf.h) fija
   `G6_VA_BASE = GSP_VA_BASE + 0x80000000` y
   `G6_VA_LIMIT = GSP_VA_BASE + 0x280000000`: diferencia de 8 GiB.
   [gsp_buf.c](../lxdde/ports/nouveau/gsp_buf.c) rechaza nuevas reservas que
   sobrepasen ese extremo.
2. **Tablas y slots:** hay 96 tablas VMM y 512 slots G6. Los pesos grandes se
   mapean en páginas de 2 MiB; una tabla PD0 cubre 512 MiB. Mapear 16 GiB
   requiere unas 32 tablas PD0, más niveles superiores, hojas pequeñas y
   bring-up. Alternar pesos grandes y pequeños puede gastar muchas hojas:
   ampliar VA sin presupuestarlas puede trasladar el fallo a las tablas.
3. **Liberación y contabilidad:** `gsp_vram_return()` descuenta `used` pero no
   recupera extents ni retrocede `next`. La reutilización de slots libres en
   `gsp_buf_alloc()` no vuelve a incrementar `used`. Los fallos tras reservar
   y los cambios de tamaño pueden consumir espacio efectivo pese a declarar
   memoria libre. Hay que distinguir bloques reutilizables, libres y perdidos
   por fragmentación, y definir una única contabilidad coherente.
4. **Cero libre mal propagado:** `vram_free_bytes()` en el driver Rust acepta
   la cifra del pool sólo si es mayor que cero; con pool agotado cae en
   `vram_total - vram_used`, que puede volver a anunciar capacidad inexistente.
5. **Planner sin coste completo:** `total_model_vram_bytes()` suma payloads,
   mientras G6 redondea reservas y VAs. Scratch y crecimiento de runtime
   también necesitan margen. El userspace mantiene otra cuenta por bytes
   solicitados. El criterio «cabe entero» debe concordar con reservas reales.

## Implementación propuesta, en orden

### 1. Identificación y telemetría fiables

- Corregir el ID/nombre GA104/3080 en `gsp_chip.*` y los tests que esperan
  GA107. Separar nombre del chip, familia de arranque y directorio de firmware;
  conservar la ruta Ampere funcional y comprobar los blobs ga104/ga102 antes
  de cambiar su selección.
- Retirar la deducción de 4 GiB para `249c`. No sustituirla por 16 GiB fijos:
  existen variantes y el tamaño debe proceder del hardware. Si falta una
  fuente fiable, expresar capacidad desconocida y evitar calcular WPR con
  una cantidad inventada.
- Propagar el nombre real de RM a userspace. Mostrar VRAM física, pool tras
  reservas, bytes asignables, límite VA, tablas y slots disponibles; un cero
  real del pool debe llegar como cero.
- Corregir metadatos de la placa y la información vigente de docs/skills.
  Conservar logs históricos y migrar/aliasar `ga107-igpu` si se renombra el ID
  de la matriz, para no perder evidencia.

Aceptación: mismo equipo identificado como GA104/RTX 3080, 16384 MiB físicos;
si aún no se ha hecho el paso 2, el diagnóstico explica el techo de 8192 MiB.

### 2. Ampliar G6 y presupuestar sus tablas

- Dimensionar la ventana para cubrir el pool completo más huecos de
  alineación. Primera configuración para esta tarjeta: **32 GiB de VA** a
  partir de la base actual, sin reservar 32 GiB de memoria física.
- Derivar límites y comprobaciones del layout, validando ausencia de
  solapamientos con GR, staging, DMA y canales. Mantener las direcciones de
  pushbuffers dentro del límite GPFIFO; la propuesta sigue por debajo de 1 TiB.
- Dimensionar tablas según el peor caso admitido y separar la arena VA de
  buffers pequeños de los pesos con páginas grandes. Mantener la exclusión
  entre PTE grande y hoja pequeña. Evitar aumentar `GSP_VMM_MAX_PT` a ciegas.
- Conservar soporte GP100/Ampere y VER3/Blackwell. Auditar cálculos de bytes,
  direcciones y parámetros CE/QMD para que no trunquen a 32 bits. Las subidas
  pueden seguir troceadas en los lotes DMA actuales.

Aceptación: prueba host de asignaciones y traducciones que cruza 4, 8 y 12 GiB
y llega cerca del extremo utilizable, con física y VA altas verificadas.
Un hardware simulado de 4 GiB sigue limitado por su pool físico.

### 3. Hacer estable el asignador bajo carga y descarga

- Gestionar extents físicos y VA reutilizables, con división/coalescencia o
  una política equivalente de slots cuyo presupuesto refleje exactamente lo
  que se puede volver a reservar.
- Resolver el descuento/reuso de `used`, el rollback completo ante errores
  de mapeo y la devolución de tablas/rangos cuando corresponda.
- Compartir el coste de asignación con el planner, o exponer una consulta de
  presupuesto que incorpore redondeos, fragmentación, tablas y slots. Distinguir
  falta de VRAM, de VA, de tablas y de slots en la telemetría.

Aceptación: repetidos ciclos de carga/liberación con tamaños distintos no
reducen la capacidad efectiva ni anuncian más memoria de la disponible.
Los fallos inyectados dejan el asignador consistente.

### 4. Confirmar residencia real de Mistral en `askd`

La inspección del índice local arroja 291 tensores: 65 F32, 193 Q4_K y 33 Q8_0.
Aplicando las reglas actuales del planner son **4.714.348.544 bytes = 4495,95
MiB = 4,39 GiB**. Redondear cada reserva como G6 suma **4727,02 MiB**;
añadiendo huecos VA en el orden del índice resulta **4854,02 MiB**. Esta última
cifra es una simulación de ese orden, no el consumo medido de `askd`.

- Identificar hash/versiones de kernel, `soso-llm`, manifest e índice de la
  imagen ejecutada. El log run9 no demuestra que lleve los cambios locales
  recientes de residencia/eager upload, ni que su índice sea idéntico al local.
- Integrar/revisar esos cambios ya presentes en `plan.rs`, `runtime.rs`,
  `user/soso-gpu` y `user/soso-llm`: no rehacerlos desde cero.
- Presupuestar payload cuantizado, padding y scratch antes de declarar
  residencia. Contabilizar KV en el dominio donde realmente se almacene.
- Diferenciar «cabe según el plan» de «subida terminada y buffers vivos»;
  sólo marcar residencia efectiva tras comprobar todas las subidas previstas.
  Replanificar de forma explícita si falla alguna reserva.
- Evitar que el refresh periódico confunda la VRAM ya ocupada por el propio
  modelo con una pérdida de capacidad que exija expulsarlo.

Aceptación: con el índice revisado, pesos elegibles cargados una vez,
contadores de uploads/evictions estables durante decode y la siguiente
pregunta, y cómputo GPU confirmado por operación. Medir primera respuesta,
prefill, decode y E/S de modelos; `backend GPU` solo no acredita residencia.

### 5. Validación en host y en la ROG

- Hostcheck de GSP: identificación, layout completo, páginas grandes/pequeñas,
  límites >8 GiB, agotamiento de tablas/slots y asignador tras fallos/liberación.
- Tests del planner con el perfil mixto Q4/Q8 de Mistral y presupuestos cerca
  del límite, incluyendo overhead y memoria ya poseída por el modelo.
- Compilar port y kernel siguiendo `soso-dev`; ejecutar las regresiones
  pertinentes de GPU. QEMU software valida contratos, no acceso al silicio.
- En la ROG, reservar buffers por un total claramente superior a 8 GiB;
  escribir/leer patrones distintos antes y después de ese umbral y cerca del
  final del pool. Ejecutar matvec con pesos en la región alta y comparar con
  CPU. Usar bloques moderados: no requiere un tensor individual de 16 GiB.
- Repetir carga/descarga y preguntas de `askd`. No basta Mistral solo para
  demostrar acceso a >8 GiB porque sus pesos ocupan menos.
- Cerrar con `halt` y comprobar `unload=ok ... dma=off`, según el procedimiento
  GPU del proyecto. Registrar el nuevo log, versiones y métricas en la matriz.

Resultado esperado: el driver puede emplear prácticamente todo el pool FB
disponible tras sus reservas, sin el techo artificial de 8 GiB; Mistral
mantiene sus pesos previstos residentes cuando su presupuesto lo permite.
La velocidad final debe medirse, no deducirse de la capacidad.

## Alcance de esta revisión

Se revisaron código, cambios locales existentes, referencias de NVIDIA/Linux,
el log run9 y el índice Mistral local.

### Implementado (host, 2026-09-14)

| Ítem | Cambio |
|------|--------|
| GA104/3080 | `gsp_chip.*`, hostcheck, matriz `ga107-igpu` |
| Ventana G6 32 GiB | `gsp_buf.h`, hostcheck >8 GiB |
| Tablas/slots | `GSP_VMM_MAX_PT=192`, banda pequeña G6 |
| Asignador | free-list coalesce, cap 512, stress hostcheck |
| Planner | `g6_reserve_bytes`, presupuesto G6 en `plan.rs` |
| Telemetría | `GpuInfo::vram_pool_free`, `g6_pt_free` |
| Userspace | `SysGpu` relee presupuesto del kernel |
| Fallbacks | sin 8 GiB inventados en `gpu.rs` / `gsp_bringup.c` |

Pendiente en placa: buffers >8 GiB, ask con `on_gpu=1`, `halt` limpio, matriz
`carga_real` / `compute_cpu_gpu` / `apagado_limpio`.

No hace falta implementar memoria compartida de Windows ni migración RAM↔VRAM
para recuperar estos 16 GiB dedicados. Ese sería otro proyecto para modelos
que excedan el framebuffer: el DMA desde sysmem ya existente es una base,
pero no sustituye un gestor de residencia compartida.
