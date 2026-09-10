# Plan de mejoras pendientes de soso

Revisión: **10 de septiembre de 2026**, versión **0.2.2**,
commit **`1df51e817` más cambios locales**. Alcance: árbol actual, plan anterior,
pruebas existentes y registros ROG hasta **run12**. Run10 no es la última evidencia.

Este plan contiene únicamente implementación o validación pendiente. Las entregas
ya implementadas del ciclo B se han retirado; su historial permanece en Git y en
[ESTADO](docs/ESTADO.md). Un cambio presente en el árbol no se vuelve a proponer
como desarrollo, aunque todavía necesite una prueba física. Las conclusiones de
lectura de código se distinguen de los resultados observados en placa.

## Prioridades y dependencias

| Orden | Entrega pendiente | Prioridad | Depende de | Cierre verificable |
|---|---|---|---|---|
| 1 | C1 host: MAC CSR 0x380, scan v14–17 fijo, UID/fin | hecho | — | `./scripts/l6-iwl-fw-hostcheck.sh` (scan_abi) |
| 2 | C2 host: contrato HCMD | hecho | — | hostcheck hcmd_contract |
| 3 | C3. Validar canal Ampere en placa (no metal en este ciclo) | P0 canal | Nueva ejecución ROG | CE y cálculo numérico en GA107 |
| 4 | C4 host: parser por arranque/hash | hecho | — | `cargo test -p xtask hw_matrix::` |
| 5 | C5 host: registro PCI `Box` | hecho | — | `cargo test -p xtask pci_stable::` |
| 6 | C6 host+QEMU: split mprotect, mremap, preflight ELF | hecho | — | `elf_mmap_rules` + `xtask test -- --guest sys --only init` (SMP=2) |
| 7 | C7 host+QEMU: marca `pid=N` y sondeo init | hecho (live pendiente) | — | `ssh_sosh_ready` exige `pid=`; arranque QEMU OK |
| 8 | C8. Asociación y datos WiFi reales | P1 | C1 + C2 + placa | AP → WPA2 → DHCP → SSH |
| 9 | C9. Perfil check + campaña | parcial | live vs minimo | `SOSO_CHECK_PROFILE`; QEMU/E2E |

Primera iteración: C1/C2 y prueba del canal de C3. C4 debe estar listo antes de
registrar nuevos resultados como validación. C5–C7 son correcciones independientes.
La estimación del plan anterior ya no aplica: estimar compute Ampere y asociación
completa después de superar sus puertas de canal/scan, sin prometer un plazo de
bring-up a partir de tests simulados.

## C1. MAC y scan: corregir las diferencias que siguen en el árbol

**Evidencia.** [run12](target/usb-diagnostic-2026-09-09-run12/SOSOLOG.txt) alcanza
ALIVE, INIT_COMPLETE y NVM_GET_INFO, pero falla SCAN_CFG. Imprime como MAC
`80:00:00:01:00:01`. Los siguientes defectos se han encontrado en el código actual;
no se atribuye a ellos, sin nueva prueba, el timeout completo de run12.

**Trabajo pendiente.**

1. Corregir las bases de lectura de MAC en
   [iwl_internal.h](lxdde/ports/iwlwifi/iwl_internal.h) e
   [iwl_mvm_nvm.c](lxdde/ports/iwlwifi/iwl_mvm_nvm.c). Se usan offsets
   `0x000/004/008/00c` desde `CSR_BASE=0`, que incluyen registros de interrupciones.
   Linux configura `mac_addr_from_csr=0x380` para 22000/AX210 y suma esos offsets
   a esa base. Seleccionar la base por dispositivo, comprobar MAC unicast y
   rechazar respuestas NVM truncadas; no anunciar «NVM listo» tras un fallo.
2. Corregir el constructor v14–17 de
   [iwl_mvm.c](lxdde/ports/iwlwifi/iwl_mvm.c): usa flags antiguos
   `PASS_ALL=BIT(2)` e `ITER_COMPLETE=BIT(5)`. La API nueva usa respectivamente
   `BIT(1)` y `BIT(2)`; `BIT(5)` significa MATCH.
3. Corregir tamaño y offsets de esa petición: el código coloca periodic/probe
   detrás de los 21 canales elegidos; Linux declara `channel_config[67]` para
   `iwl_scan_channel_params_v7` y envía la estructura completa. El campo count
   expresa canales válidos, no desplaza las estructuras posteriores. Comprobar
   todos los offsets, campos reservados y tamaño contra la API v15/v17.
4. Mantener una lista explícita de versiones implementadas; rechazar una versión
   futura o desconocida en vez de tratar cualquier `>=14` como compatible.
   Construir canales a partir del perfil NVM/MCC y distinguir activo/pasivo;
   no usar siempre la misma lista y el contexto de canal 6.
5. Modelar fin normal, aborto y timeout de scan. Cero BSS con fin normal es un
   resultado vacío; BSS parciales sin fin no acreditan un scan completado. Usar
   UID y cancelar o recuperar antes de iniciar el siguiente intento.

**Referencia local:** [cfg/22000.c](lxdde/linux/drivers/net/wireless/intel/iwlwifi/cfg/22000.c),
[iwl-csr.h](lxdde/linux/drivers/net/wireless/intel/iwlwifi/iwl-csr.h),
[fw/api/scan.h](lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/scan.h) y
[mvm/scan.c](lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/scan.c).

**Aceptación.** Un banco llama al constructor real y compara bytes/offsets con
valores de la referencia, sin reutilizar su función de tamaño como oráculo.
MMIO simulado distingue los registros de MAC de CSR_INT/INT_MASK. Cubrir scan
vacío, abortado, tardío, repetido y versión no soportada. En AX200: MAC contrastada
con inventario fiable, SCAN_CFG aceptado, fin normal y BSS del AP de prueba.

## C2. Respuestas, propiedad de slots y recuperación de HCMD

**Evidencia de código:** [iwl_trans.c](lxdde/ports/iwlwifi/iwl_trans.c),
`handle_gen2_rx`, `iwl_trans_send_cmd` y `iwl_trans_send_cmd_wait`.
Existe espera de respuesta, pero `cmd_status=1` significa coincidencia de secuencia;
no se comprueban grupo/opcode ni rechazo del comando. El envío reemplaza el único
pending y avanza módulo 32 sin control de slots pendientes. Smart Fifo se envía sin
esperar su resultado en [iwl_mvm_up.c](lxdde/ports/iwlwifi/iwl_mvm_up.c).

**Trabajo pendiente.**

1. Separar respuesta recibida, error de transporte y resultado del firmware.
   No completar HCMD con una notificación espontánea ni con una respuesta tardía
   de otro comando; validar flags, identidad y payload según cada API.
2. Definir serialización y propiedad del pending/slot entre poll, init, scan y
   tráfico. No sobrescribir un comando vivo. Tras timeout, drenar o reiniciar
   la cola antes de reutilizar secuencias que puedan recibir respuestas antiguas.
3. Validar longitud con aritmética amplia antes de sumar la cabecera, copiar o
   modificar el estado; limitar cada paquete RX al buffer DMA real. Auditar TB0,
   visibilidad DMA y descriptores contra `pcie/tx-gen2.c`.
4. Esperar y verificar SF, propagar errores de SHARED_MEM_CFG/NVM/MCC que impidan
   continuar y definir qué estado se revierte si falla una fase intermedia.

**Aceptación.** Ampliar [tools/iwl-hostcheck](tools/iwl-hostcheck) con rechazo FW,
respuesta ajena, notificación con índice coincidente, timeout y respuesta tardía,
más de 32 envíos, dos solicitantes y tamaños extremos. Ninguno completa el comando
equivocado, pisa DMA pendiente ni declara MVM listo tras una etapa obligatoria fallida.

## C3. Canal Ampere y contrato de compute por familia

**Evidencia.** Run12 recibe `GSP_INIT_DONE` y crea vaspace; RM devuelve
`NO_MEMORY (0x51)` al reservar clase `0xc56f`, con `chid=0`. El cambio a
`idx + GSP_CHAN_RSVD_CHIDS` ya está en
[gsp_chan.c](lxdde/ports/nouveau/gsp_chan.c): queda **validarlo**, no reimplementarlo.
NO_MEMORY no demuestra por sí solo que falte VRAM ni confirma que el chid sea
la única causa. El [diagnóstico](docs/DIAGNOSTICO-ROG-2026-09-09.md) conserva las ejecuciones.

**Trabajo pendiente.**

1. Probar el canal con el kernel que incluya el cambio actual; registrar clase,
   chid, USERD, instance block, method buffer, flags y respuesta RM. Si persiste
   el rechazo, comparar el payload completo y sus reservas/mapeos con nouveau
   r570, conservando la distinción entre rechazo explícito y timeout.
2. Superado RM_ALLOC, validar copia CE y readback antes de habilitar compute.
3. Introducir capacidades por familia para QMD, métodos y código máquina:
   [gsp_compute.c](lxdde/ports/nouveau/gsp_compute.c) construye `GspQmdV05`
   incondicionalmente; [l6-g4f-build-sass.sh](scripts/l6-g4f-build-sass.sh)
   genera un único juego de blobs, por defecto `sm_120`. Elegir una clase de
   compute del catálogo no valida que esos comandos/blobs sirvan en Ampere.
4. Empaquetar variantes con metadatos de arquitectura, registro y constant bank;
   implementar el encoder Ampere contra su referencia. Rechazar combinaciones
   incompatibles antes del submit, manteniendo disponible lo ya validado de CE.

**Aceptación.** Host: payload de canal y encoders por familia con referencias
independientes, rechazo de blob incompatible. Placa: GA107 supera canal, CE,
saxpy/matvec comparados con CPU y después una carga pequeña real. Revalidar GB205
tras cambios compartidos y comprobar apagado con objetos, unload, halt y DMA.

## C4. Resultados por ejecución, etapa y dispositivo

**Evidencia:** [hw_matrix.rs](xtask/src/hw_matrix.rs) conserva búsquedas sobre el
log completo. `gsp` + cualquier `ok` puede acreditar RPC; `vram` + `mib` acredita
pool sin crearlo; «generado» acredita carga GPU aunque se haya usado CPU.
`asociad` admite incluso «no asociado». La fusión conserva etapas antiguas cuando
la nueva ejecución no llega a ellas, sin separar resumen histórico y estado actual.

**Trabajo pendiente.** Parsear eventos por línea, arranque, intento y backend;
separar ALIVE, INIT_COMPLETE, MVM listo, scan, asociación y WPA2. Guardar resultado
por ejecución con hash del log, identidad del kernel/binarios/firmware desplegados,
PCI y condiciones de prueba. Conservar el historial, pero no presentar un éxito
antiguo como evidencia del kernel nuevo. No copiar etapas GPU a una entrada WiFi
ni DHCP Ethernet a WiFi. Derivar los contadores de arranque de criterios explícitos.

**Aceptación.** Fixtures de run10/run12 y casos mínimos: `GSP firmware cargado` +
`memtest OK`, VRAM detectada sin pool, CPU que genera texto, «no asociado», DHCP
Ethernet, dos boots concatenados y log truncado tras una ejecución verde.
Ninguno acredita una etapa que no sucedió. El historial sigue siendo consultable.

## C5. Direcciones estables en el registro PCI de lxdde

**Evidencia:** [kernel/src/lxdde/pci.rs](kernel/src/lxdde/pci.rs) almacena
`LxPciDev` en `Vec` y entrega `last_mut()` como puntero C. Los drivers conservan
ese puntero; un `push` posterior puede realojar el vector. Es un riesgo de vida
útil comprobable en código, no una explicación demostrada del cuelgue de run10.

**Trabajo pendiente.** Dar a cada dispositivo almacenamiento estable, por ejemplo
`Box<LxPciDev>`; definir liberación de un probe fallido y remove sin dejar referencias
en drivers. Mantener el probe fuera del lock, que ya está implementado.

**Aceptación.** Forzar crecimiento del registro después de un probe que conserve
su puntero; comprobar identidad, drvdata e IRQs de los dispositivos anteriores.
Ejercitar fracaso de un probe y retirada sin invalidar a los restantes.

## C6. Memoria parcial y validación ELF consistente

**Evidencia:** [syscall.rs](kernel/src/task/syscall.rs),
[addrspace.rs](kernel/src/task/addrspace.rs) y [elf.rs](kernel/src/task/elf.rs).
El cambio de PTE y el shootdown existen; persisten estos casos:

- `sys_mprotect` actualiza `MmapRegion::writable` solo si cubre la región entera.
  Un subrango perezoso puede volver con los permisos anteriores al fault-in.
- `grow_anon` no exige que `old_len` coincida con la región ni valida allí el final
  contra el límite mmap; separa comprobar solapes y modificar el mapa. El caso
  `new_len == old_len` devuelve éxito antes de validar flags o región.
- `load` no comparte las comprobaciones de `load_lazy`; este último acepta entry
  en el hueco entre segmentos y solo comprueba `page_delta <= offset`, sin verificar
  la congruencia de offset/vaddr. También ignora errores al materializar páginas BSS.

**Trabajo pendiente.** Dividir regiones al cambiar permisos parciales y mantener
coherentes PTE, fault-in y páginas grandes; validar y reservar el mapa de forma
atómica entre hilos. Validar rango/alineación/longitud real antes de cualquier
mremap, incluso si no cambia de tamaño. Unificar preflight ELF previo a mutaciones:
aritmética comprobada, `filesz <= memsz`, congruencia, entry dentro de un PT_LOAD
ejecutable y política explícita de solapes. Propagar OOM sin mapa parcial.
Mantener el rechazo explícito de shrink, PROT_NONE y otros modos no soportados;
implementarlos no es necesario para cerrar esta entrega.

**Aceptación.** QEMU SMP: proteger una página interior aún no materializada y
otra ya presente; comprobar permisos tras reclaim/refault y en mapeos grandes.
Mremap con longitud inventada, límites, colisión concurrente y OOM conserva el
mapa previo. Ambos loaders rechazan entry en hueco, overflow y segmento incongruente;
un fallo al reservar BSS aborta la carga sin dejar un proceso parcialmente construido.

## C7. Confirmación OTA cuando sosh ya funciona

**Evidencia.** Run12 imprime `sosh viva sin /tmp/sosh-ready; no confirmo OTA`
y después procesa comandos. [sosh](user/sosh/src/main.rs) ya prefaulta y escribe
la marca, pero ignora resultados de mkdir/write/close;
[init](user/init/src/main.rs) espera una ventana finita y luego no vuelve a
comprobar la marca en esa ejecución. El log no permite decidir entre binario
antiguo, error de escritura o latencia de arranque.

**Trabajo pendiente.** Identificar kernel y rootfs realmente desplegados; registrar
fallo de creación/escritura/cierre y tiempos de prefault/marca. Usar una señal de
readiness ligada a esa instancia de sosh y continuar comprobándola sin bloquear
la supervisión de PID 1. Confirmar solo cuando la señal sea válida y el proceso
siga vivo; no sustituirlo por un timeout mayor o por detectar el banner.

**Aceptación.** Live/QEMU con arranque lento, escritura fallida, marca vieja y
muerte después de spawn. La shell sana acaba confirmando una OTA pendiente;
las demás no. Verificar en placa la candidata y conservar el resultado del shim.

## C8. Asociación y camino de datos WiFi reales

**Evidencia:** [iwl_mvm_assoc.c](lxdde/ports/iwlwifi/iwl_mvm_assoc.c) marca
`associated=1` después de MAC_CONTEXT/ADD_STA, sin intercambio de autenticación y
asociación con el AP. `iwl_mvm_connect_open` elige el primer BSS si no encuentra
el SSID solicitado; `iwl_mvm_tx_8023` envía el frame como HCMD de control.
Ya existe [wifi_wpa.rs](kernel/src/net/wifi_wpa.rs); no hace falta crear otro supplicant.

**Trabajo pendiente.** Elegir exactamente SSID/BSSID/canal, interpretar RSN y
estado abierto; implementar MLME y enlace MAC/PHY/station según la API del blob,
con estado asociado solo tras respuesta válida del AP. Separar colas TX de datos
de HCMD, encapsulación/decapsulación 802.11 y completions. Conectar EAPOL/CCMP y
claves al supplicant existente, y limpiar enlace/DHCP al perder asociación.

**Aceptación.** AP controlado: conexión abierta y WPA2, clave errónea, SSID ausente,
deauth, reconexión y tráfico bidireccional. No conectar a otra red como fallback.
Obtener lease WiFi y abrir SSH sobre esa interfaz; validar contadores y captura
en el AP, no solo logs del driver.

## C9. Integración y campaña de validación pendiente

1. Ejecutar la candidata exacta en ROG después de C1/C2; validar el cambio de chid
   y HCMD wide/DQA ya presentes. Guardar nuevas ejecuciones sin sobrescribir run12.
2. Integrar las regresiones nuevas en las suites existentes. Endurecer
   [check.rs](xtask/src/check.rs) por perfil: hoy un script obligatorio ausente se
   omite. La ausencia de hostcheck/firmware exigido por live debe fallar; un perfil
   mínimo puede omitirlo de forma explícita. Usar lockfiles sin actualizarlos al validar.
3. Completar artefactos del job host/build y la selección manual de suites:
   [.github/workflows/check.yml](.github/workflows/check.yml) ya ejecuta QEMU y
   conserva logs, pero `suite=ambas` no incluye `qemu-shards`. Definir y probar
   la selección anunciada, sin recrear los jobs existentes.
4. Tras las correcciones: `cargo xtask check`, integración QEMU y E2E live
   instalación/actualización/resize. Tres arranques por equipo y una sesión de
   al menos 30 minutos con la función ejercitada; registrar errores y latencias.
5. Persistencia física: probar cortes sobre medios desechables que soporten el
   flush requerido, verificando hashes y GPT al recuperar. El pendrive ROG anuncia
   SYNCHRONIZE CACHE no soportado: comprobar rechazo de resize, no eludirlo.
6. Steam Deck: seguir únicamente los pendientes del
   [plan específico](docs/PLAN-STEAMDECK.md): primer arranque para confirmar
   IOMMU/rotación/entrada y transporte IPCR/QMI, HTC/WMI y datos de ath11k.
   Su codec QMI y MHI ya escritos requieren integración/validación; no rehacerlos
   ni mezclar sus resultados con AX200. Esta rama no bloquea cerrar la ROG.

Cada entrega actualiza solo la documentación afectada, sincroniza las skills de
dominio al cambiar comportamiento y elimina de este plan los puntos completados.
Las optimizaciones de rendimiento se programan después de obtener corrección y
una medida reproducible; no se añaden nuevas familias GPU ni un port de rustc a este ciclo.

## Verificación de esta revisión

Revisión estática del árbol y lectura de registros ya copiados; no se ha flasheado
ni vuelto a ejecutar la placa. Solo se modifica este plan. Comprobaciones ejecutadas:

- `scripts/l6-iwl-fw-hostcheck.sh`: pasa; log `/tmp/soso-astra-plan-iwl.log`.
- `scripts/l6-g3-gsp-hostcheck.sh`: pasa; log `/tmp/soso-astra-plan-gsp.log`.
- `cargo test -p xtask hw_matrix::`: **13 tests pasan**; log
  `/tmp/soso-astra-plan-matrix.log`.

Estos resultados validan las pruebas existentes, no los escenarios de placa.
`SOSO_QEMU_SMP=2 cargo xtask test -- --guest sys --only init` pasó (marca
`pid=` + init test con mprotect interior y `old_len` inventada). Quedan C3/C8
y la parte live de C7/C9. La comparación de ABI usa los fuentes Linux locales.
