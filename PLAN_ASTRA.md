# Próximo ciclo de mejoras de soso

Fecha: **7 de septiembre de 2026**. Versión: **0.2.2**.
Base revisada: commit **`9048b12f4`** y árbol local, con 13 archivos modificados
previamente en ELF/mmap, iwlwifi, nouveau, hostcheck, empaquetado y build.
Horizonte: **3–4 semanas**, suponiendo una persona dedicada.

## Objetivo

Consolidar las capacidades añadidas recientemente para poder trabajar desde
soso sin comprometer los datos ni confundir una operación iniciada con una
operación terminada. Priorizar almacenamiento, contratos de memoria/procesos
y el bucle de desarrollo remoto antes de ampliar arquitecturas de modelos o
intentar portar todo rustc.

Este documento **sustituye el plan A1–A9 anterior**, cuya implementación queda
como base del ciclo. Ya existen el aislamiento de sesiones SSH del arnés,
el cliente HTTPS con reloj y control de origen, la enumeración PCI cacheada,
`cargo xtask check`, el workflow de CI, metadatos de recuperación del kernel,
cierre explícito de staging y la matriz hardware. No se propone crearlos otra vez.
Las entregas siguientes se identifican como **B1–B6** para distinguirlas de ese
trabajo. Las referencias a A1–A9 que queden en documentación son históricas.

## Prioridades y alcance

| Orden | Entrega | Prioridad | Esfuerzo estimado | Dependencias |
|---|---|---|---|---|
| 1 | B1. Redimensionado recuperable y persistencia real | P0 | 4–6 días; contención inicial ≤1 día | Ninguna |
| 2 | B2. Contratos de memoria, ELF y argumentos | P1 | 4–6 días | Ninguna; integrar los cambios locales de ELF |
| 3 | B3. Bucle Forja que compile los fuentes del guest | P1 | 3–5 días | B2 para argumentos sin pérdidas; B1 si exige ampliar disco |
| 4 | B4. CI que compruebe los componentes obligatorios | P1 | 1–2 días | Se amplía con las pruebas de B1–B3 |
| 5 | B5. Evidencia hardware sin falsos positivos | P1 | 1–2 días de herramientas/banco | Hardware disponible para cerrar etapas físicas |
| 6 | B6. Quitar lecturas GPT del camino de datos | P2 | 1–2 días | B1: contrato de actualización de geometría |

El conjunto suma **14–23 días de trabajo**, sin contar bring-up nuevo ni esperas
por hardware. El compromiso de cuatro semanas es B1–B4 y el parser de B5; B6 y
la campaña completa de placa entran si queda margen. La primera entrega de B1
debe impedir operaciones inseguras aunque la recuperación completa requiera
más tiempo. No condicionar esa contención al final del ciclo.

## B1. Redimensionar sin dejar el disco a medio mover

**Hallazgos en el código.**

- [kernel/src/fs_resize.rs](kernel/src/fs_resize.rs), `grow_root`, reduce el
  superbloque de modelos y desplaza p3/p4 antes de actualizar GPT. Los registros
  de fase se escriben después de esas operaciones. `journal_write` devuelve
  éxito si falta `SOSORES.TXT`; no se encontró un lector de ese journal ni una
  recuperación al arrancar en kernel o boot-shim.
- [kernel/src/drivers/live_disk.rs](kernel/src/drivers/live_disk.rs),
  `disk_slide_sectors`, copia hacia atrás sector a sector. Una fase global y el
  desplazamiento no bastan para reanudar una copia solapada interrumpida:
  parte del origen puede haber sido sobrescrita por el destino.
- `LiveRootDev::flush` y `LiveModelsDev::flush` devuelven `Ok(())` sin actuar.
  Virtio sí hace flush en su escritura de sector, pero esa propiedad no se
  puede extender al resto de backends. Los tests con dispositivos de memoria
  no demuestran persistencia ante pérdida de alimentación en USB/NVMe.

**Trabajo acotado.**

1. Validar toda la geometría, el espacio libre y la disponibilidad del journal
   antes de la primera escritura. Rechazar el redimensionado mientras no se
   pueda garantizar su recuperación. Detectar importaciones o usuarios activos
   de modelos y definir una exclusión que abarque toda la operación.
2. Extraer planificación y recuperación a código comprobable en host. Elegir
   entre traslado fuera de línea o copia reanudable con progreso durable y
   espacio auxiliar. Documentar el orden real **p1 → p2 → p4 → p3**, los límites
   de cada partición y los puntos en que se puede continuar o volver atrás.
3. Persistir intención y progreso antes de las mutaciones que los necesitan;
   recuperar antes de montar volúmenes afectados. No repetir una copia
   solapada completa como si fuera idempotente.
4. Dar un contrato explícito de persistencia a cada backend y propagar sus
   errores. Conectar las barreras necesarias a datos, superbloques, GPT y
   journal. Si un dispositivo no puede cumplirlo, devolver una limitación
   explícita para esta operación.

**Aceptación.** Usar imágenes pequeñas desechables con patrones distintos en
rootfs, SOSOINSTALL y modelos. Inyectar cortes durante la copia, escrituras
parciales, errores de flush y cambios de GPT; tras recuperar, comprobar los
hashes de los tres contenidos y la coherencia de ambas tablas GPT. Cubrir
journal ausente/corrupto, espacio insuficiente y una importación activa.
Antes del diseño completo, todos los casos no soportados deben fallar sin
escribir. Validar después el ciclo completo en QEMU live; la prueba de pérdida
de alimentación real se registra por backend, sin deducirla del resultado host.

**Estado (sept 2026).** Implementado: crate `soso-resize-core` (journal, slides
reanudables, recovery GPT), recovery en arranque (`fs_resize::recover_before_mount`),
preflight (journal, import activo, flush durable, geometría), `soso-resize` userspace,
21 tests host en `cuts.rs`. `cargo xtask test-resize` OK en QEMU live (KVM,
`drv-live-disk`, tiny 256 MiB): grow `rootfs +32K` y recovery tras corte
`SHRINK_MODELS` (slide+GPT; finalize no re-crece si el montaje ya estiró
sosofs). USB: `SYNCHRONIZE CACHE(10)` (IMMED=0) al enumerar; si CSW OK,
`backend_supports_durable_flush` es cierto y `disk_flush` lo reenvía. NVMe:
opcode Flush del namespace. Sin sonda USB o sin CSW OK, resize sigue
`ENOTSUP`. QEMU `test-usb` (KVM, 2026-09-09): `qemu-xhci` y `nec-usb-xhci`
llegan a `sync_cache=true` (imagen live por escenario, no compartida).
`test-usb` kbd/hub: una sola sesión HMP para `sendkey` (antes se
reconectaba por tecla). Los cuatro escenarios OK (KVM, 2026-09-09).
Si `flush` falla al terminar un slide, `slide_done` no avanza (host
`slide_flush_error_does_not_commit_progress` / GPT
`apply_gpt_flush_error_keeps_journal_phase`). No sustituye un corte de
alimentación en placa.

## B2. Hacer efectivos los contratos de memoria y procesos

**Hallazgos en el código.**

- [AddrSpace::set_prot](kernel/src/task/addrspace.rs) no modifica páginas:
  solo comprueba el rango. `sys_mprotect` puede devolver éxito sin retirar
  permiso de escritura; tampoco representa `PROT_NONE` ni permisos de
  ejecución en esa ruta.
- `grow_anon` cambia la longitud registrada antes de completar el mapeo y no
  comprueba allí colisiones con otras regiones. `sys_mremap` ignora flags y
  devuelve éxito para una reducción sin reducir el mapeo.
- [read_spawn_args](kernel/src/task/syscall.rs) convierte `argv` en
  `parts.join(" ")`: pierde argumentos vacíos y los límites de argumentos que
  contienen espacios. `envp` también se transforma en texto separado por
  saltos de línea. Ambos recorridos necesitan límites globales y aritmética
  comprobada para sus tablas de punteros.
- [load_lazy](kernel/src/task/elf.rs) ya está cambiando localmente para
  resolver cabeceras y páginas compartidas de segmentos. Queda por comprobar
  explícitamente `filesz <= memsz`, los límites de fichero, la congruencia de
  offset/dirección y el punto de entrada. Recortar un segmento al tamaño del
  fichero no equivale a rechazar un ELF truncado.

**Trabajo acotado.**

Separar la entrega en dos pasos revisables. Primero, memoria: implementar el
subconjunto soportado de `mprotect`/`mremap`, mantener coherentes PTE y regiones
perezosas, invalidar TLB en los cores que comparten el espacio y deshacer una
reserva fallida. Rechazar explícitamente flags o modos todavía no soportados.
Conservar los ajustes locales de ELF y añadir validación previa a la creación
de regiones, con una política clara para BSS, permisos y páginas compartidas.

Después, procesos: conservar `argv`/`envp` como datos delimitados hasta la
entrada al programa. Mantener un adaptador para la ABI histórica de argumentos
en texto, sin volver a introducir esa pérdida en la nueva. Establecer límites
de cantidad y bytes; revisar el contrato de TLS de proceso/hilo antes de usarlo
como base de libstd.

**Aceptación.** En QEMU con SMP, una escritura tras retirar permiso mata solo
al proceso de prueba, y un cambio permitido vuelve a funcionar. Ejercitar
páginas ya presentes y todavía perezosas, colisión, reducción y falta de RAM
en `mremap`, comprobando que un fallo conserva el mapa anterior. Un hijo debe
recibir exactamente `['', 'a b', 'ñ', 'x=y']` y el entorno previsto. ELFs
truncados o malformados se rechazan sin panic del kernel; un ELF válido con
tabla de secciones fuera de la cabecera inicial sigue arrancando. Añadir las
regresiones a `init test` y mantener el shard con presión de memoria.

**Estado (sept 2026).** En árbol: `mprotect`/`mremap` con PTE+TLB, `read_spawn_args`
y `read_spawn_env` con tablas delimitadas (límites 256×4096 B), codificación
`SOSA` en pila, validación ELF en `load`/`load_lazy`. Regresiones B2 en
`init test`: mprotect, mremap (grow/shrink/colisión), argv, envp (`x=y`), ELF
malformado y truncado (cabecera parcial de `/bin/init`). **Validado en QEMU**
(`cargo xtask test -- --guest sys --only init`, KVM, 2026-09-09). Shard sys
completo y los demás shards de `cargo xtask test` siguen como puerta de
integración (B4).

## B3. Cerrar un bucle de desarrollo remoto demostrable

**Hallazgos en el código.**

- [user/soso-forja/src/main.rs](user/soso-forja/src/main.rs), `cmd_sync`,
  envía rutas y hashes, pero no contenidos. El servidor guarda ese manifiesto
  en memoria y `run_release` compila su propio checkout: una edición del guest
  no se convierte por ese camino en un cambio de los fuentes del build.
- [tools/soso-forja-server/src/main.rs](tools/soso-forja-server/src/main.rs)
  lee una primera porción de 4096 bytes y, para `/sync`, espera EOF con
  `read_to_end`. El cliente espera la respuesta manteniendo abierta la
  conexión: hay una espera circular potencial independiente de la compilación.
  Tampoco se encuadran las peticiones por `Content-Length`.
- El cliente elimina cabeceras sin validar el estado HTTP; `cmd_install`
  ignora el resultado del hijo y llama a `halt`. El servidor puede lanzar
  builds simultáneos sobre el mismo árbol y publica por `VERSION`, sin
  identificar una petición concreta.
- `cmd_local` calcula hashes; `cmd_build_local` copia artefactos previamente
  compilados. [soso-rustc](user/soso-rustc/src/main.rs) declara que es un stub.
  Esa infraestructura es útil, pero todavía no compila Rust dentro de soso.

**Trabajo acotado.**

Hacer fiable primero la ruta remota: protocolo incremental de manifiesto →
contenido necesario → verificación de hashes → build identificado. Trabajar
en un directorio separado por sesión, con rutas normalizadas y sin escribir
sobre el checkout de desarrollo. Definir altas, cambios y borrados, y qué
ocurre si se corta una sincronización.

Leer cabeceras y cuerpos completos con límites, timeout y tratamiento binario;
responder al terminar el cuerpo, sin esperar el cierre del cliente. Propagar
errores HTTP, de escritura/cierre y del actualizador. Serializar builds o
aislar sus salidas y entregar un conjunto consistente de pack, manifiesto y
kernel con identidad de fuentes. El servidor escucha hoy en `0.0.0.0`:
ofrecer bind configurable y autenticar las operaciones que suben fuentes o
arrancan builds antes de usarlas en una red compartida.

**Aceptación.** Modificar desde el guest un mensaje de `hola-std`, sincronizar,
compilar en host, aplicar y arrancar el binario que muestra el nuevo mensaje.
Su artefacto debe estar ligado al hash de los fuentes enviados. Probar con
transporte simulado peticiones fragmentadas, cuerpos mayores de 4096 bytes,
datos no UTF-8, HTTP 500, timeout, rutas inválidas y clientes concurrentes.
Un fallo de build o instalación no anuncia éxito ni apaga la máquina.

Mantener documentada la distinción entre planificación local, copia de
artefactos y compilación remota. El primer compilador nativo real queda como
hito posterior a B2/B3, con una prueba que compile y ejecute un programa;
`--version` no sirve como criterio de cierre.

**Estado (sept 2026).** Servidor: HTTP con `Content-Length` (cuerpos binarios,
timeout, fragmentación), sync con hashes SHA-256, rutas normalizadas (rechazo
de `..`), borrados respecto al manifiesto previo, builds serializados (`503`),
árbol en `target/forja-work` o `SOSO_FORJA_WORK`, bind `SOSO_FORJA_BIND`,
token `SOSO_FORJA_TOKEN` (obligatorio si el bind no es loopback; Bearer).
Cliente: `--token`; no trata 4xx/5xx como éxito; un
fallo de `soso-update` no hace `halt`. Artefacto identificado: `forja-id` /
`X-Forja-Build-Id` / `GET /build-id` ligados al hash de los fuentes. Demo
host `hola-std`: sync de un mensaje nuevo → pack con esa identidad; con
`SOSO_FORJA_RELEASE=hola-std` el pack es el ELF y contiene el mensaje
(`hola_std_demo_identified_artifact`, `hola_std_compiles_synced_message`).
15 tests host. **Demo guest en QEMU** (`cargo xtask test -- --guest sys --only forja`):
`write-hola` → sync → build identificado → `apply` del ELF → `/bin/hola-std`
muestra el mensaje nuevo. Un pack ELF no llama a `soso-update` ni hace `halt`.
`0.0.0.0` sin `SOSO_FORJA_TOKEN` no arranca. Con token, POST y GET
(incluidos pack/manifest/kernel/`/build-id`) exigen Bearer.

## B4. Completar la puerta de integración existente

**Hallazgos en el código.** El workflow
[.github/workflows/check.yml](.github/workflows/check.yml) ya llama a
`cargo xtask check`, pero no ejecuta QEMU ni publica los logs. En
[xtask/src/check.rs](xtask/src/check.rs), un fallo de `build_boot_shim` se
convierte en aviso y no incrementa los fallos. La instalación de targets en
el workflow termina con `|| true`. Los tests de Forja y otras herramientas
nuevas no están incluidos en las listas host de `check`.

**Trabajo.** Mantener el comando común y hacer obligatorios los componentes
necesarios para cada perfil, incluido el shim en live/release. Separar un
prerrequisito ausente de un error de compilación. Incorporar las pruebas de
Forja y de las entregas B1/B2; ejecutar los shards QEMU en CI y conservar logs
incluso al fallar. Añadir una ejecución programada o manual para
`test-usb`, `test-install` y `test-update`, con OVMF y sus demás requisitos
declarados. Usar lockfiles y evitar alterar dependencias durante la validación.

La recuperación OTA ya tiene helpers y pruebas: ampliar la cobertura para
ejercitar el **orden de operaciones del shim real**.
[aplicar_kernel](boot-shim/src/actualiza.rs) copia el backup y marca
`BackupReady` antes de `Applying`. Los tests de
[recovery.rs](crates/soso-update-core/tests/recovery.rs) cubren corte antes
y a medias del backup (no restauran). Queda el corte entre la meta
`Probando` y el buzón, y el fallo de una shell perezosa después de que
`spawn` haya devuelto PID.

**Aceptación.** Un error de shim, una regresión Forja y un fallo QEMU hacen
fallar sus jobs. Los resultados indican qué perfil se probó y qué quedó sin
ejecutar. La candidata a release tiene logs de instalación y actualización
con sus escenarios de fallo, sin convertir un aviso en una comprobación
superada. La disponibilidad de WiFi/GPU física no bloquea los jobs host/QEMU.

**Estado (sept 2026).** `cargo xtask check` falla si falta el shim o los tests
de Forja/`soso-resize-core`. El workflow ya no usa `|| true` en los targets.
Jobs aparte: `qemu-sys` (init en QEMU, timeout 90 min) y `qemu-shards`
(4 shards TCG, `SOSO_TEST_JOBS=1`, timeout 360 min) en cada PR y push;
`e2e-live` (`test-usb`/`test-install`/`test-update`) en `workflow_dispatch`
y cron semanal; logs como artefactos. `workflow_dispatch`/`shards` y el
cron también corren los 4 shards.
El shim ya no escribe `Applying` antes de completar el backup del kernel
(un corte a medias no marca recuperación sobre el hueco del kernel nuevo).
Tests host: `fault_before_backup_must_not_restore`,
`fault_during_partial_backup_must_not_restore` y corte `Probando`→buzón
(`after_interrupt`: completa el buzón, no reaplica). Init no marca `OK` hasta que sosh deja `/tmp/sosh-ready` y `kill(pid, 0)`
sigue vivo (no basta el PID de `spawn` ni 50 ms). sosh falta las páginas
PT_LOAD y construye el lector **antes** de la marca; init espera ~400 ms
más de sondeo por si el proceso muere al instante. Si el hijo vive sin
marca, la shell sigue y el buzón no pasa a `OK`. Un page-fault de un
binario **lanzado después** (no de sosh) sigue fuera de esta confirmación.
`kill(pid, 0)` es sondeo
(`SIGPROBE`); cubierto en `init test`. `qemu-sys` (`cargo xtask test -- --guest sys --only init`) corre en
cada PR y push (timeout 90 min, TCG en Actions) y comprueba
`/tmp/sosh-ready` por SSH. `cargo xtask check`
incluye `xhci-nostd` (CDB SYNC CACHE) y corre `hw_matrix::` con
`-p xtask` desde la raíz. `e2e-live` sigue en `workflow_dispatch` y cron
semanal. `cargo xtask test-update` (KVM, 2026-09-09): aplicar, versión,
recuperación `applying` y manifiesto `..` OK; SSH corta al ver la marca
(`ssh_guion_hasta`) porque `halt` no cierra la sesión. El paso sys
«SSH + halt» (`ssh_sesion`) usa el mismo corte (token `soso_ssh_ok_42`)
en vez de esperar 60 s a que el cliente se cuelgue. `test-install` (KVM):
copia + shim `Boot0009` + arranque solo NVMe OK. Las comprobaciones host
miran el fin **más alto** de la GPT (p3 detrás de p4) y magic `SOSOFS11`.
`DISK_FLAG_SOSO` acepta también `SOSOFS10` (reinstalar un disco viejo).
`cargo xtask check` corre `test_install::ultima_llega_al_final` (p3 detrás de p4).
`e2e-live` sigue en `workflow_dispatch` y cron semanal (OVMF, timeout 360 min; no bloquea
el merge si WiFi/GPU física no está).

## B5. Evitar que la matriz hardware certifique un fallo

**Hallazgos en el código.** La matriz ya existe; hace falta mejorar la evidencia
que registra. En [xtask/src/hw_matrix.rs](xtask/src/hw_matrix.rs):

- `parse_gpu_stages` acepta `vram` + `pool`, incluso en `pool VRAM=no`.
- Considera `unload=fallo` junto a `dma=off` un apagado correcto.
- Un comando `soso-llm run` basta para marcar carga real, y el dispositivo
  software puede marcar la comparación CPU/GPU como correcta.
- `parse_wifi_stages` acepta palabras como `wpa2` o `reconnect` sin distinguir
  intento de resultado; el banner de sosh basta para marcar SSH.
- [docs/hw-matrix.json](docs/hw-matrix.json) conserva etapas pendientes,
  firmware sin hashes y un PCI ID provisional. No acredita una nueva pasada
  de hardware sobre el commit actual.

**Trabajo.** Parsear resultados explícitos por línea y por arranque, con estados
`ok`, `fail`, `pendiente` y `no_aplica`. Dar prioridad a fallos de la misma etapa;
no mezclar mensajes de intentos o dispositivos distintos. Vincular cada
resultado a log, commit, identificación de cambios locales, chip y firmware.
No sobrescribir evidencia anterior con el estado de otro arranque.

**Aceptación.** Fixtures negativos para todos los ejemplos anteriores y para
logs truncados o con intentos fallidos seguidos de recuperación. Ninguno marca
éxito sin el evento correspondiente. Usar la herramienta corregida para
revalidar los equipos disponibles: tres arranques y una sesión sostenida,
con tráfico WiFi o resultado numérico GPU según el dispositivo. Corregir los
datos provisionales solo a partir de inventario real. El trabajo de bring-up
que aparezca se estima aparte; una etapa no probada queda pendiente.

**Estado (sept 2026).** Parser estricto: `pool VRAM=no`, `unload=fallo`,
`soso-llm run` solo, dispositivo software, `wpa2`/`reconnect`/`wifi scan` y
el banner `sosh —` ya no marcan `ok`. Estados `fail` / `no_aplica`. Fixtures
negativos y log truncado en `xtask` (`hw_matrix::`). `parse-logs` fusiona
por etapa: un arranque truncado no baja un `ok`/`fail` a `pendiente`; un
`fail` o `ok` nuevo sí actualiza. Hashes SHA-256 del firmware **del árbol**
(`rootfs/lib/firmware`) en `hw-matrix.json` (ucode AX211/AX200; GSP gb205;
ga107 usa el blob ga102 de fallback). No acreditan una pasada en placa.
PCI de GA107: `10de:249c` (ROG 3050 Mobile, inventario de
[`docs/L5c-on-box.md`](docs/L5c-on-box.md)); `collect --pci` y los tests
rechazan huecos `????`. `g1-check --vfio-hint` no imprime `echo 10de ????`
si `lspci -n` no da un id concreto. Las etapas de placa siguen `pendiente` hasta
revalidar: tres arranques y una sesión sostenida.

## B6. Reducir el coste de E/S del disco live

**Hallazgo en el código.** `LiveRootDev::part` y `LiveModelsDev::part` llaman a
`current_part` en [live_disk.rs](kernel/src/drivers/live_disk.rs). Cada llamada
lee de nuevo la cabecera GPT y cuatro sectores de entradas, incluso en
consultas de capacidad y operaciones ordinarias de bloques. Esto añade E/S
de metadatos al camino de lectura de modelos y de escritura del rootfs.

**Trabajo.** Mantener una geometría validada en memoria con actualización
explícita tras un cambio GPT confirmado. Integrarla con B1 para que ningún
lector use una geometría intermedia. Medir antes de introducir otras cachés.

**Aceptación.** Un backend instrumentado verifica que las lecturas ordinarias
no vuelven a leer GPT y que, tras actualizar la geometría, usan los límites
correctos. Comparar lecturas físicas, MiB/s y tiempo hasta primer token en
frío/caliente con la misma imagen y modelo; guardar los resultados. No fijar
un objetivo de tok/s sin una línea base ni atribuir a GPU una mejora de disco.

**Estado (sept 2026).** `LiveRootDev`/`LiveModelsDev` usan geometría en RAM
(`refresh_geometry` tras GPT de resize/recovery). `GeomCache` en
`soso-resize-core` (21 tests host: lookup no relee GPT; reload ve límites
nuevos). `space_info` lee la caché, no la tabla. Medido en la misma
`soso-live.img` (8 MiB de p3, 2026-09-09): releer GPT por sector
557056 lecturas / 35.9 MiB/s; con caché 16384 lecturas / 1283 MiB/s.
Disco en RAM 4 MiB: 211 vs 11056 MiB/s. Guest sobre la **misma**
`soso-live.img` (OVMF + virtio-blk + `live-disk`, KVM, 2026-09-09, CPU, no
GPU): `tiny` en p3. Frío: carga 0 ms / 2 peticiones; generado 8 tokens /
40 ms; disco 44 pet / 13 ms. Caliente (otro proceso): carga 10 ms / 0
peticiones; 8 tokens / 30 ms; disco 42 pet / 5 ms. Mismo protocolo con
`bench` (161 MiB, hidden=1024, 4 capas) en p3 de 256 MiB, CPU, no GPU
(logs `target/test-b6/ssh-*-bench.txt`). Frío: carga 0 ms / 2 pet; 8
tokens / 580 ms; disco 1626 pet / 207368 KiB / 285 ms (caché 0 %).
Caliente: carga 0 ms / 0 pet; 8 tokens / 860 ms; disco 1616 pet /
206848 KiB / 462 ms (caché 100 %). La carga no relee GPT; el generate
sigue leyendo ~202 MiB de pesos. No fijar objetivo de tok/s ni atribuir
el tiempo a GPU.

## Secuencia propuesta

- **Semana 1:** contención de B1, contrato de persistencia y pruebas de cortes;
  hacer estricto el resultado de shim en B4.
- **Semana 2:** B2, empezando por memoria y carga ELF; terminar o mantener
  deshabilitadas explícitamente las modalidades de resize aún inseguras.
- **Semana 3:** B3, con una demostración completa desde una edición del guest;
  integrar sus pruebas en B4 y corregir el parser de B5.
- **Semana 4:** margen para integración, validación por equipo y B6 si las
  entregas prioritarias están cerradas.

Cada entrega actualiza las secciones afectadas del manual y las guías de
arquitectura/desarrollo/live. `docs/ESTADO.md` debe apuntar al ciclo B y reflejar
los límites verificados. No esperar al cierre del mes para documentar un
comando deshabilitado o un cambio de ABI.

## Fuera del compromiso de este ciclo

Port completo de rustc/cargo/gix, enlazador dinámico general, multiusuario,
IPv6, nuevas familias GPU, nuevas arquitecturas de modelos y optimización de
modelos de 70B sin medidas. El rollback integral del rootfs sigue siendo una
limitación publicada: este ciclo no promete resolverlo ni amplía por ello la
compatibilidad OTA entre kernels y userspace arbitrarios.

## Evidencia y límites de esta revisión

Revisión dirigida de código y documentación locales: plan anterior e historial,
arquitectura/desarrollo/live, memoria y syscalls, Forja/self-hosting, disco live,
actualización, CI y matriz hardware. Los problemas descritos son hallazgos de
lectura estática salvo que se indique una ejecución. No son una auditoría
exhaustiva ni una afirmación de fallo reproducido en placa.

Se ejecutó la siguiente selección de pruebas host, sin descargar dependencias:

```sh
cargo test --offline \
  -p soso-update-core -p sosofs -p sosomfs -p gptdisk \
  -p soso-forja-server --features std
```

Resultado: **correcto en los cinco paquetes; comando terminado con código 0**.
Esto valida las pruebas existentes, no los escenarios nuevos propuestos arriba.
El log de esta sesión está en `/tmp/soso-astra-host-tests.log` y no es un
artefacto versionado. También se comprobaron los enlaces locales del plan y
`git diff --check -- PLAN_ASTRA.md`.

No se ejecutaron `cargo xtask check`, suites QEMU/USB/instalación/OTA, VFIO ni
pruebas físicas en esta revisión. No se modificó código de implementación ni
los cambios locales preexistentes para preparar el plan.
