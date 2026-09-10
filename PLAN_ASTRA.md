# Plan de mejoras pendientes de soso

Revisión: **10 de septiembre de 2026 (tarde)**. Base: **`382b4113f`**.

Esta revisión sustituye la de la mañana. Entre las dos se ha implementado todo
lo que se puede comprobar **sin placa**: bancos host, tests de xtask y QEMU con
llamadas reales. Lo que necesita hardware (o un AP controlado) sigue pendiente y
se dice cuál es exactamente.

Última evidencia física: **run15/run16**, en
[usb-diagnostic-2026-09-10-run3](target/usb-diagnostic-2026-09-10-run3/SOSOLOG.txt)
y siguientes; kernel anunciado `0.2.2 (276404696-dirty)`. Ninguna de las
entregas de esta revisión se ha ejecutado en placa.

«Implementado», «comprobado sin hardware» y «validado en placa» son tres
estados distintos y aquí no se mezclan.

## Estado por entrega

| Entrega | Estado | Qué lo respalda |
|---|---|---|
| R1. Banco HCMD fiable + sanitizadores | **Cerrada** | `hcmd_contract`, ASan+UBSan en los tres bancos C |
| R2. Identidad de arranque e importación | **Cerrada** | 27 tests de `hw_matrix::`, fixture run15, matriz remigrada |
| R3. Candidata identificable | **Parcial** | `SOSOHASH.TXT` + `hw-matrix artefactos`; falta placa |
| R4. Propiedad de cola HCMD y recuperación | **Cerrada** | `hcmd_queue` (5 casos) + candado en Rust |
| R5. CE y compute por familia | **Parcial** | juegos SASS sm_86/sm_120, rechazo pre-submit; medidas en placa pendientes |
| R6. Canales y scan | **Cerrada (host)** | `mcc_chan`, `scan_abi`; scan físico pendiente |
| R7. Asociación real | **Parcial** | `bss_select` (RSN, SSID exacto); 802.11 con AP pendiente |
| R8. WPA2 y conectividad | **Pendiente** | necesita AP WPA2 |
| R9. Contrato de mremap | **Cerrada** | `init test` en QEMU, 9 casos nuevos |
| R10. CI y campaña | **1–2 cerradas; 3–5 pendientes** | `cargo xtask check` derivado del perfil |

## Lo cerrado en esta revisión (no se vuelve a proponer)

**R1.** El banco construía paquetes de 64 B y anunciaba 476: ASan lo detectaba.
Los fixtures usan el tamaño real, se comprueba el contenido NVM v4 entero
(468 B) y no solo la longitud, y `handle_gen2_rx` recibe los bytes realmente
disponibles: si la longitud anunciada los supera, descarta en vez de recortar en
silencio. `cmd_resp_wire_len`/`cmd_resp_trunc` distinguen respuesta completa de
recortada y NVM/MCC rechazan las recortadas. Los tres bancos C
(iwlwifi, GSP, ath11k) se compilan y ejecutan también con ASan+UBSan; eso
destapó un desplazamiento con UB en el fixture de ath11k.

**R2.** El corte del log en `boot: memtest` dejaba el banner de versión —seis
líneas antes en run15— en una «ejecución» aparte, y cualquier mención a SOSOKRN
valía como versión. Ahora el arranque empieza en su banner, las cabeceras de
instantánea no abren ejecución y sin banner la identidad es explícitamente
desconocida. Las fuentes se correlacionan por tipo (SOSOLOG/serie son logs;
SOSODRV/SOSOWIFI son informes del último arranque). `arranques_consecutivos_ok`
se deriva del historial ordenado, así que reimportar no mueve la racha y un
intento fallido posterior la corta. Cada ejecución separa «llegó a userspace» de
«superó la campaña» y guarda los hashes de los artefactos. `hw-matrix migrar`
rehízo el historial: 14 → 28 ejecuciones, todas con banner.

**R4.** La cola de comandos es FIFO estricta con propiedad por slot
(libre/sync/async/respondido/envenenado). Los async mantienen su slot hasta su
respuesta, el DMA se recicla solo en orden, la cola llena da backpressure en vez
de sobrescribir y un envío reentrante se rechaza. Tras un timeout el slot queda
envenenado y la cola bloqueada: `iwl_trans_recover()` + recarga de firmware es
el único camino, y `iwl_ax211_scan/connect` lo hacen solos. En Rust, un candado
único serializa envío/RX/sondeo entre cores.

**R6 (host).** MCC_UPDATE valida versión de notificación, status y tamaño exacto
(cabecera + 4·n_canales) y aplica la lista; sin eso `lar_regdom_set` sigue a 0.
Una sola función selecciona canales para v6 y v14–17 distinguiendo NVM ausente
(lista mínima **solo pasiva**), perfil válido sin canales (no se escanea) y
perfil aplicado. Y corrige la codificación por canal: pasivo es FORCE_PASSIVE
(bit 26), no el bit 0 del mapa de SSID directos, y en v17 la banda va en
`flags[31:30]`, no en el byte que esa versión usa como `psd_20` — el AX211
anuncia scan v17.

**R9.** `sys_mremap` documenta y aplica el contrato (flags 0, dirección
alineada, longitud cero es EINVAL, longitudes normalizadas al alza como en
mmap/munmap, encoger EINVAL). `grow_anon` reserva el intervalo bajo el candado
del libro de regiones y materializa las páginas con el candado suelto; si falla
a medias, el rollback devuelve la longitud anterior y retira solo sus páginas.
Comprobado con llamadas reales en QEMU, incluido un hijo `mremap-oom` que pide
más páginas de las que hay (tamaño calculado con `meminfo`).

**R10.1–2.** `cargo xtask check` ejecuta **todos** los tests de xtask (el filtro
dejaba fuera `pci_stable::` y `elf_mmap_rules::`), guarda la salida de cada paso
en `target/check-artefactos/` y la vuelca al fallar, y deriva los hostchecks
obligatorios del perfil compilado: si el perfil trae ath11k su banco no se
puede omitir por falta del script, y se comprueba que el firmware que ese port
necesita está en el rootfs salvo que el perfil lo excluya. El perfil por defecto
conserva la exigencia previa (iwlwifi + nouveau) y el mínimo sigue sin exigir
nada.

## Prioridades de lo que queda

| Orden | Pendiente | Prioridad | Necesita |
|---|---|---|---|
| 1 | R3. Validar la candidata exacta en placa | P0 | ROG + USB |
| 2 | R5.1/R5.2. Por qué muere el CE tras GR0 | P0 | ROG (GA107) |
| 3 | R5.5. Descriptor QMD de Ampere | P0 | cabecera `clc?c0qmd.h` V02_xx |
| 4 | R6.2. Scan físico repetible | P1 | AX200/AX211 + AP |
| 5 | R7.2. Autenticación/asociación 802.11 y colas de datos | P1 | AP controlado |
| 6 | R8. WPA2 y conectividad sostenida | P1 | AP WPA2 |
| 7 | R10.3–5. Campaña de cierre | P1 | las anteriores |

## R3. Validar la candidata exacta y resolver la divergencia del rootfs

**Hecho sin placa:** `package-usb-live` escribe `SOSOHASH.TXT` en la ESP con
version, build, perfil, features y el sha256 de kernel, ESP, rootfs y modelos;
`hw-matrix parse-logs --sosohash` ata esa identidad a la ejecución importada, y
`hw-matrix artefactos` la registra desde el árbol. Así una ejecución deja de
identificarse solo por la versión anunciada, que no distingue dos árboles.

**Pendiente (placa):** flashear la candidata, conservar la ejecución nueva sin
pisar run15/run16, y comprobar en ese orden:

1. TX_ANT transmitido con grupo 1 y su respuesta; después cada etapa MVM y el
   primer scan. No declarar resuelto el timeout porque la cabecera coincida.
2. Run15 imprime `sosh viva sin /tmp/sosh-ready; no confirmo OTA` y el init
   actual imprime otra cosa: con `SOSOHASH.TXT` en la ESP eso ya se puede
   atribuir a un userspace anterior o descartar.
3. `hw-inv: no pude escribir /etc/soso-hw (Io)`: si persiste con la candidata
   coherente, aislar escritura/cierre/persistencia del rootfs.

**Cierre:** artefactos identificados por hash y primer fallo localizado. Para
C7 físico: marca del PID vivo, confirmación de una OTA pendiente y resultado del
shim.

## R5. GPU: continuidad de CE y contrato compute por familia

**Hecho sin placa:**

- Tabla de capacidades por familia (clase, versión de QMD, arquitectura de
  SASS) y **un juego de SASS por arquitectura**: `sass_sm86.c` (GA107) y
  `sass_sm120.c` (GB205), generados con sus metadatos del cubin y su
  manifiesto. Compilar para otra arquitectura ya no sobrescribe la anterior.
- El driver elige el juego por la familia deducida de la clase de compute (o
  del chip si no hay catálogo) y **rechaza antes del submit** si la familia no
  tiene descriptor QMD escribible, si el SASS es de otra arquitectura o si el
  blob no está en VRAM.
- Los juegos no son intercambiables y ahora se demuestra: el mismo `.cu` deja
  los parámetros en `param_base=352` con sm_86 y en 896 con sm_120.
- Las tres medidas de CE (antes de GR0, tras crearlo, tras PROMOTE_CTX) con la
  misma copia de 4 KiB, y volcado de estado en el primer fallo: los dos canales
  con GET/PUT del PBDMA, semáforo esperado frente al real y drenaje de
  fault/RC. La precarga de SASS antes de GR0 sigue, pero **después** de la
  primera medida: es mitigación, no diagnóstico.

**Hallazgo que cambia el trabajo pendiente:** el árbol solo trae
`cla0c0qmd.h`, que llega a **V01_07 (era Pascal)**. El layout de QMD de
Ampere/Ada/Hopper no está, así que esas familias declaran `qmd_version = 0` y no
se les envía nada. Los métodos sí coinciden (`clc7c0.h`: `SET_OBJECT` 0x0000,
`SEND_PCAS_A` 0x02b4, `SEND_SIGNALING_PCAS2_B` 0x02c0), luego lo que falta es
exactamente el descriptor.

**Pendiente:**

1. Conseguir el `clc?c0qmd.h` de Ampere (V02_xx) o su equivalente
   documentado, transcribir los campos como se hizo con `QMDV05_*` y validar el
   encoder contra esa cabecera en el banco. Sin eso, GA107 no lanza compute
   por decisión, no por accidente.
2. Ejecutar las tres medidas de CE en GA107 y leer el resumen
   (`sonda CE — antes de GR0=…, tras crear GR0=…, tras PROMOTE_CTX=…`). Eso
   dice si el CE muere al crear el canal de GR o en el promote, que es lo que
   run15 no permitía distinguir. Comparar con
   [fifo/r535.c](lxdde/reference/linux-6.15-nouveau/drivers/gpu/drm/nouveau/nvkm/engine/fifo/r535.c)
   y [gr/r535.c](lxdde/reference/linux-6.15-nouveau/drivers/gpu/drm/nouveau/nvkm/engine/gr/r535.c).
3. Corregir la causa: pesos, entradas y readback necesitan COPY0 **durante** la
   ejecución; un readback anterior a GR0 no acredita compute.
4. saxpy/matvec ejecutados de verdad y comparados con CPU dentro de
   tolerancias; después carga pequeña real y regresión GB205.

## R6. Scan físico (lo que falta de R6)

**Pendiente (placa + AP):** en AX200/AX211, AP controlado visible y varios
scans con UID/fin consistentes. El resultado ya se propaga con causa distinta
para vacío normal, aborto, timeout, sin canales y sin regdominio (`wifi scan`
lo dice), y `sys_wifi_scan` devuelve 0 —no EIO— cuando el scan acaba bien sin
redes.

## R7. Asociación real y transporte de datos WiFi

**Hecho sin placa:** un BSS se clasifica por su beacon —SSID, canal del DS
Params, bit Privacy y RSN IE (versión, cifrados, AKM)— y no por suposición;
antes todo se marcaba abierto. `iwl_mvm_pick_bss` elige el SSID exacto y, con
varios BSS del mismo nombre, el de mejor RSSI, y fija su canal. `connect_open`
rechaza una red protegida y `connect_wpa2` rechaza la que no ofrezca WPA2-PSK
con CCMP en vez de derivar a la ruta abierta.

**Pendiente (AP controlado):** autenticación y asociación 802.11 con respuesta
válida del AP antes de anunciar enlace; contextos MAC/PHY/station y colas TX de
datos con completions y encapsulación/decapsulación. Hoy `tx_8023` sigue
enviando un frame como HCMD de control y `iwl_mvm_assoc_prepare` marca asociado
tras MAC_CONTEXT/ADD_STA: eso no es un enlace y no se puede arreglar a ciegas.
Comparar con [mlme.c](lxdde/linux/net/mac80211/mlme.c) y MVM TX/RX/STA.

**Cierre:** AP abierto controlado, tráfico bidireccional y captura en el AP;
SSID ausente, rechazo y deauth no dejan estado asociado.

## R8. WPA2 y conectividad sostenida

Sin cambios: la ruta del kernel ya entra por `connect_wpa2` del driver (que
exige RSN/PSK/CCMP), pero el 4-way, la instalación/retirada de claves y la
autorización de datos siguen pendientes de AP. Invalidar asociación, claves y
lease al perder enlace, y recuperar sin reiniciar.

**Cierre:** AP WPA2 con clave correcta e incorrecta, rekey, deauth y
reconexión; DHCP/SSH atribuidos a WiFi y tráfico sostenido 30 minutos.

## R10. Integración y campaña de cierre

1. **Hecho.** Todos los tests de xtask en `check`, artefactos por paso y
   sanitizadores en los tres bancos C.
2. **Hecho.** Hostchecks y firmware obligatorios derivados del perfil.
3. **Pendiente.** Para cerrar candidata: `cargo xtask check`, QEMU SMP y E2E
   live instalación/OTA/resize con los artefactos exactos de R3, guardando
   ejecuciones nuevas sin sobrescribir run15/run16.
4. **Pendiente (placa).** Tres arranques distintos por equipo y sesión de
   30 minutos con las funciones ejercitadas. Separar CPU/GPU, Ethernet/WiFi y
   arranque frío/reinicio; registrar resultados, errores y latencias.
   `hw-matrix record-boot` ya deriva la racha del historial, así que anotar un
   arranque a mano no infla contadores.
5. **Pendiente.** Steam Deck conserva su [plan específico](docs/PLAN-STEAMDECK.md);
   esa rama no bloquea la validación ROG.

## Verificación de esta revisión

| Comprobación | Resultado |
|---|---|
| `./scripts/l6-iwl-fw-hostcheck.sh` (con ASan+UBSan) | Pasa: contract, queue, BSS/RSN, MCC, scan ABI, DQA, TLV |
| `./scripts/l6-g3-gsp-hostcheck.sh` (con ASan+UBSan) | Pasa: 120 OK, juegos sm_86/sm_120 y rechazo por familia |
| `./scripts/l6-ath11k-hostcheck.sh` (con ASan+UBSan) | Pasa: 74 comprobaciones |
| `cargo test -p xtask` | Pasa: 69 tests (27 de `hw_matrix::`, 7 de `check::`) |
| `cargo xtask test sys --only "init test"` | Pasa: batería de syscalls con los 9 casos de mremap |
| `cargo xtask check` | **TODO OK** (host, builds, tres hostchecks, tests xtask) |
| `cargo xtask test` (4 shards) | 23 OK, 3 FALLO: los dos `ask` de la línea base y `A7` |
| `cargo xtask build` (perfil por defecto y `live-usb`) | Compila |

Sobre los tres fallos de la suite: los dos de `ask` son la línea base conocida
(la sesión SSH no cierra; el comando sí se ejecuta y su salida se ve en el
stdout del fallo). El tercero, `soso-llm: 20 ciclos carga/generación/cambio
(A7)`, falla por lo mismo —«la sesión SSH no terminó en 1200 s» con los ciclos
completados dentro del guest— y es un paso **añadido el 2026-09-07**
(`9048b12f4`), posterior a la línea base registrada, sin ninguna ejecución
verde conocida. El perfil por defecto de la suite no compila lxdde, así que
nada de WiFi/GPU de esta revisión entra en ese kernel; de lo que sí entra,
`mremap` solo lo usa `init test`, que pasa.

No se ha flasheado ni ejecutado la placa. Las comparaciones usan los fuentes
Linux/NVIDIA locales; los bancos host no acreditan funcionamiento físico de
scan ni de compute.
