# ROG RTX 3080 Laptop: diagnóstico (10 oct 2026)

## Evidencia y alcance

Lectura de la ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copia en
`target/usb-diagnostic-2026-10-10/`. ESP desmontada al terminar. El PSK de
`SOSOWIFI.TXT` no sale de esa copia.

| Campo | Valor |
|---|---|
| Kernel arrancado | **soso 0.3.7 (3588d3d34)** — commit del 8 oct, mensaje `T22` |
| Manifiesto `SOSOHASH.TXT` | Sigue en el empaquetado del 5 oct 07:49: `version=0.3.7`, `build=cd5f6a36a-dirty`, `perfil=nouveau,iwlwifi` |
| Flush / uptime | **#633**, **5189295 ms** (~86 min), sin panic |
| PCI | `10de:249c` GA104, `8086:2723` AX200, `10ec:8168` rtl8169, `1002:1638` framebuffer GOP |
| Rootfs live | `fs: sosofs live (generación 69)`, `task: /bin/init lanzado (pid 1)`, `sosh: marca lista pid=2` |
| Instalación | `soso-install nvme1 --force` copió **4500 / 4500 MiB** y dejó `SOSOBOOT.TXT` con `INSTALL` de la ESP `FBC6D1C0-…` en nvme1 |
| Cierre | `soso-update aplicar` agotó el plazo al conectar; después `halt` |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | `2a402fb04afa` (6.6.32) — `r8169_main.c`, `vmmgf100.c` |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf6ac0c` — `r535/bar.c` |

Hostcheck (exit 0 los dos): `scripts/l6-iwl-fw-hostcheck.sh` y
`scripts/l6-g3-gsp-hostcheck.sh`, salida en
`target/usb-diagnostic-2026-10-10/`. No cubren el transporte USB, el TCP de
esta placa ni el CDN de GitHub.

No se validó en esta sesión: un saxpy con resultado numérico, inferencia,
SSH, reconexión WiFi, ni el arranque desde el NVMe ya clonado.

## Tabla de etapas

| Etapa | Cita SOSOLOG | Resultado |
|---|---|---|
| Shim | `soso-shim: UEFI alcanzado`, `Boot0001 soso` | OK |
| Userspace | `boot: task`, `sosh: marca lista pid=2` | OK |
| GPU GSP | `GSP_INIT_DONE`, `GSP=rm_compute, pool VRAM=sí` (16070 MiB) | OK |
| GPU CE | `CE selftest OK`, `CE readback verificado (G4e GO)` | OK |
| GPU compute | `compute listo cls=0xc7c0 … G6 listo` | Listo; sin saxpy ejecutado |
| WiFi | `UCODE_ALIVE_NTFY`, `alive=true`, scan `count=21 end=1` | OK |
| WPA2 | `4-way completado`, `asociado a 'Rutilo' aid=3` | OK |
| DHCP | `net: backend lx-wifi`, `net: dhcp 192.168.68.132/24` | OK |
| Ethernet | `phystatus 0x84 … enlace DOWN` | Sin cable |
| Instalar | primer `soso-install` **exit 1**; con `--force`, `100%  4500 / 4500 MiB` | Copia hecha; el rechazo inicial es un bug |
| Arranque UEFI del NVMe | `Petición de arranque anotada en SOSOBOOT.TXT`; no hubo reinicio con el USB | Pendiente de un arranque más del pendrive |
| OTA aplicar | `github.com` conecta; `185.199.111.133:443` → `EAGAIN`; `plazo agotado al conectar` | Fallo de esta sesión, sin `#PF` |
| Apagado | `halt` / `halt: apagando soso` | Pedido; sin panic en el log |

## Hallazgos

### 1. Reinstalar un disco soso exige `--force` porque p4 se llama «windows» (confirmado)

- **Síntoma:** `soso-install: nvme1 tiene particiones de otro sistema: p4 windows`, `uso: soso instalado`, exit 1. El listado ya muestra `p4 windows 64 MiB SOSOINSTALL`. Con `--force` y la confirmación del nombre, la copia llegó al 100 %.
- **soso:** `crates/gptdisk/src/lib.rs:446` etiqueta `EBD0A0A2-…` (tipo GPT `0700`) como `"windows"`, y `is_foreign` (`463-468`) lo cuenta como ajeno. `user/coreutils/src/bin/soso-install.rs:497-511` rechaza el destino si `foreign_partitions` no está vacío, aunque `DISK_FLAG_SOSO` haga verdadero `disk_safe`. El propio empaquetado escribe ese tipo a propósito: `xtask/src/package_live.rs:158-160` (`-t 4:0700`, `-c 4:SOSOINSTALL`) para que Linux monte el FAT. El test `etiquetas_y_ajenos` (`crates/gptdisk/src/lib.rs:740-742`) solo excluye el `0x8300` del live, no esta p4.
- **Linux:** no hay un homólogo que trate `0700` como «disco Windows intocable». Es *Microsoft basic data*, el tipo con el que el stick deja el instalador visible desde Linux. `is_foreign` promete marcar solo lo inequívoco (`lib.rs:459-462`) y aquí marca la partición que soso acaba de clonar.
- **Causal:** **confirmado** para el exit 1. No impidió la copia: el segundo intento con `--force` terminó.

### 2. La red WiFi se pierde en `soso-update aplicar` (causa en el transporte iwlwifi; ver sección «Red»)

- **Síntoma:** dos `conectado (fd 4)` a `140.82.121.3:443` (`github.com`). Luego DNS de `release-assets.githubusercontent.com`: dos respuestas `id=distinto` de 44 B y una `id=ok` de 118 B con A. `conectando a 185.199.111.133:443` acaba en `EAGAIN (errno 11)` y `soso-update: plazo agotado al conectar`. No hay `EXCEPTION` ni `#PF`.
- **soso:** el plazo de `tcp_connect` es 30 s (`crates/soso-http/src/lib.rs:642-650`). `net: poll sin candado (1 veces)` (`kernel/src/net/mod.rs:548-555`) cayó en el primer DNS, y esa conexión sí se estableció: no explica el tercer connect.
- **Causal:** las dos respuestas de 44 B no son del CDN. Son las respuestas a las consultas `github.com` `id=39346` e `id=40475` (44 B = cabecera + pregunta `github.com` + un A), que ya habían llegado con `id=ok`. El servidor DNS las contestó **otra vez** justo cuando salió la tercera consulta. El análisis de abajo lo atribuye al anillo TX del driver. El page fault abierto de `soso-update` (puntero `0x153…`) **no está** en este log.

### 3. BAR1 PDB de instancia ≠ `bar1PdeBase` — descartado

- **Síntoma:** `PDB=0x31b233f89e94b000` frente a `bar1PdeBase=0x3f3c2a000`, PD3 inválido al caminar BAR1. El mismo aviso que el 24 sep.
- **soso:** `lxdde/ports/nouveau/gsp_bar1.c:263-268`.
- **Linux:** `lxdde/reference/linux-master-nouveau/.../r535/bar.c:128-147` (`r535_bar_bar1_init`) envuelve `rm_bar1_pdb` y no programa el bloque de instancia. `gf100_vmm_join_` está en `lxdde/linux/.../vmmgf100.c:342`.
- **Causal:** **descartada**. En este arranque hay `CE readback verificado (G4e GO)`, `pool VRAM=sí` y `compute listo cls=0xc7c0`.

### 4. rtl8169 enlace DOWN — descartado

- **Síntoma:** `phystatus 0x84 bmsr 0x7989 → enlace DOWN 10M half`, con firmware PHY cargado (211 opcodes).
- **soso:** el driver arrancó; el lease salió por `net: backend lx-wifi`.
- **Linux:** `lxdde/linux/drivers/net/ethernet/realtek/r8169_main.c:4642-4651` (`r8169_phylink_handler`) solo despierta la cola si `netif_carrier_ok`.
- **Causal:** **descartada**. No hay cable. No explica el timeout del CDN ni el exit 1 del instalador.

### 5. `SYNCHRONIZE CACHE(10)` rechazado por el pendrive — descartado

- **Log:** `SYNCHRONIZE CACHE(10) falló` y `sync_cache=false` en el mass storage `048d:1234`.
- **soso:** `kernel/src/drivers/usb_storage.rs:353-356` devuelve `Ok(())` si el probe dejó `sync_cache` en falso.
- **Linux 6.6.32:** `drivers/scsi/sd.c` **sí** está en `lxdde/linux` (la primera versión de este informe decía lo contrario). `sd_sync_cache` (`sd.c:1683-1700`) devuelve éxito cuando el dispositivo contesta asc `0x20` (comando inválido) o `ILLEGAL REQUEST`. `SYNCHRONIZE CACHE(16)` solo se usa en discos zonados (`sd_zbc.c:933`). soso hace lo mismo que Linux.
- **Causal:** **descartada**. Hacer fallar `flush` aquí dejaría sin commits de sosofs el rootfs de este pendrive, que Linux sí escribe. Esta sesión montó la generación 69, lanzó `/bin/init` y clonó 4500 MiB al NVMe: no reproduce la raíz huérfana del 5 oct.

## Red: por qué se pierde

### Lo que dice el log

- El enlace funcionaba en el instante del fallo. Las tres consultas DNS salieron y se contestaron (`SOSOLOG.txt:1548-1572`), y dos sesiones TLS completas con `github.com` terminaron justo antes. No hay `dhcp perdido`.
- Lo único anómalo antes del timeout son las **respuestas DNS repetidas** a consultas viejas, que aparecen en el momento de transmitir la tercera.
- El driver no registra nada del enlace: ni desautenticación, ni pérdida de beacons, ni cola TX llena, ni fallos de `wifi_send`. Ese silencio no indica que no pasara nada. No existe código que lo detecte (puntos D y E).
- El 24 sep se vio el mismo patrón: `aplicar` con `errno 11` en `tcp_connect` y luego DNS `errno 2` (`DIAGNOSTICO-ROG-2026-09-24.md:61`).

### A. El puntero de escritura TX da la vuelta a 16, no a 256 (divergencia confirmada; causa probable)

- **soso:** las colas TX de mgmt y datos tienen 16 TFD (`IWL_MGMT_QUEUE_SIZE`). `iwl_trans_tx` envuelve el puntero en 16 y lo escribe tal cual en el doorbell (`lxdde/ports/iwlwifi/iwl_trans.c:1896-1898`, `tx_doorbell` en `1023-1028`). El puntero inicial que devuelve `SCD_QUEUE_CONFIG` se recorta a 16 (`iwl_trans.c:1779`). Al enviar la trama 16, el doorbell pasa de `15` a `0`.
- **Linux 6.6.32:** para la familia 22000, a la que pertenece el AX200, `max_tfd_queue_size = 256` (`drivers/net/wireless/intel/iwlwifi/cfg/22000.c:53`); en AX210 es 65536 (`cfg/ax210.c:72`). `write_ptr` y `read_ptr` cuentan módulo ese valor (`queue/tx.h:76-80`). La ranura física es `ptr & (n_window-1)` (`queue/tx.h:22-25`). El doorbell recibe el puntero completo (`queue/tx.c:79`, `804-805`), y el inicial del `SCD_QUEUE_CONFIG` se guarda módulo 256 (`queue/tx.c:1209`). Para una cola de 16 ranuras (`IWL_MGMT_QUEUE_SIZE`, `fw/api/txq.h:82`), Linux escribe 1…255, 0… en el doorbell, nunca 15→0.
- **Consecuencia (hipótesis):** el hardware compara punteros de 8 bits. Cuando el doorbell cae a 0 con su lectura en 15, ve 241 TFD pendientes y vuelve a recorrer las 16 ranuras con su contenido viejo. Eso reenvía al aire tramas ya enviadas. Las dos consultas DNS viejas que el servidor vuelve a contestar en `1570-1571` son justo eso. El socket DNS solo guarda 2 paquetes (`kernel/src/net/dns.rs:135`), así que dos `distinto` es el máximo visible, no el número real de repeticiones. Cada vuelta del puntero produce una ráfaga de reenvíos a 6 Mbit/s fijos (`iwl_mvm_tx.c:10-18`). También produce una respuesta `TX_CMD` por reenvío en un anillo RX de 32 RB, y `iwl_trans_tx_reclaim` (`iwl_trans.c:699-715`) mueve el consumidor según índices de TFD reenviados. El SYN y el SYN-ACK del CDN caen en esa ventana. Encaja con «Respuesta repetida» (`WIFI-OPERATIVA.md:170`) y con los 72 `TX resp` duplicados del 16 sep (`DIAGNOSTICO-ROG-2026-09-16.md:384`). Aquel caso se atribuyó solo al wrap de RX, ya corregido.
- **No refutado:** la cola de comandos (32 ranuras, mismo patrón en `iwl_trans.c:481-526`) llegó a `seq=0x0015`: 21 comandos, sin vuelta. El arranque no la ejercita.

### B. Solo se lee el primer paquete de cada RB (divergencia confirmada; efecto por medir)

- **soso:** `drain_rx_gen2` llama a `handle_gen2_rx` una vez por RB (`iwl_trans.c:1044-1072`, `902+`).
- **Linux:** `iwl_pcie_rx_handle_rb` recorre el RB con `offset += ALIGN(len, FH_RSCSR_FRAME_ALIGN)` hasta `FH_RSCSR_FRAME_INVALID` (`pcie/rx.c:1323-1348`). Solo corta tras el primero en AX210 o posterior (`pcie/rx.c:1407-1408`). En 22000 el firmware puede empaquetar varios.
- **Consecuencia:** en el AX200 se pierde en silencio cualquier respuesta `TX_CMD`, notificación o MPDU que comparta RB con otra. Por sí solo se pierden paquetes, pero el enlace no se corta. Sumado a A, pierde respuestas de reclaim.

### C. Número de secuencia 802.11 siempre 0 en datos (divergencia confirmada; efecto por medir)

- **soso:** `iwl_mvm_eth_to_80211` deja `seq_ctrl = 0` con el comentario «lo rellena el firmware» (`lxdde/ports/iwlwifi/iwl_mvm_data.c:176-182`). Envía las tramas como datos no QoS.
- **Linux:** para datos no QoS el número lo pone mac80211 desde un contador por interfaz, `+= 0x10` (`net/mac80211/tx.c:863-873`). mvm lo respeta: `mvm/tx.c:1193-1211` solo reescribe el número en QoS y con la API TX vieja.
- **Consecuencia (hipótesis):** si el firmware no lo rellena, el AP descarta como duplicada toda retransmisión MAC (`Retry=1`, mismo SN 0), después de haberla confirmado con ACK. Toda trama que necesite reintento se pierde sin que nadie lo sepa. Se comprueba con una captura en modo monitor en el canal 3.

### D. Nada consume EAPOL después del 4-way (hueco confirmado; corte a plazo fijo)

- **soso:** `four_way_handshake` (`kernel/src/net/wifi_wpa.rs`) crea el `Supplicant` en local y lo suelta al llegar a `Authorized`. Nadie vuelve a llamar a `wifi_receive_eapol`. El `handle_group` de `soso-wpa2` existe, pero no tiene llamador en el kernel. La cola EAPOL del driver tiene 4 ranuras y sobrescribe (`iwl_ax211.c:446-458`).
- **Consecuencia:** cuando el AP rota la GTK, no recibe el mensaje 2/2 del group handshake. Tras sus reintentos desautentica a la estación. Es un corte total a plazo fijo, el intervalo de rekey del AP, no ligado al tráfico. En esta sesión el fallo cae con el tráfico (en la tercera conexión), no a plazo, así que D no explica este log. Sí explica cortes «tras un rato» en sesiones largas.

### E. Ni la pérdida del enlace se detecta ni hay reconexión (hueco confirmado; hace permanente cualquier corte)

- **soso:** `iwl_mvm_rx_mlme_frame` solo trata BEACON, AUTH y ASSOC_RESP (`lxdde/ports/iwlwifi/iwl_mvm_assoc.c:111-157`). Las DEAUTH y DISASSOC se ignoran. El reparto de notificaciones (`iwl_trans.c` ~965-1009) no maneja `MISSED_BEACONS_NOTIFICATION` (0xa2). `associated` y `authorized` no vuelven a falso. `kernel/src/net/mod.rs` no tiene camino de enlace caído: `NIC_REAL` no se limpia, el DHCP no se reinicia y no se reasocia.
- **Linux:** `mvm/ops.c:355` registra el handler y `mvm/mac-ctxt.c:1557-1620` lleva a `ieee80211_beacon_loss` / connection loss. mac80211 desasocia y wpa_supplicant reconecta.
- **Además:** el `TxToken` de WiFi descarta el resultado de `wifi_send` (`kernel/src/net/device.rs:164`). Un `-1` por cola llena (`tx_full_drop`, sin log: `iwl_trans.c:1832-1837`) es una trama que smoltcp da por enviada. El anillo de entrega RX de 8 ranuras sobrescribe al llenarse (`iwl_ax211.c:432-442`).

## Orden de corrección

1. **Una p4 llamada `SOSOINSTALL` no es un disco Windows.** En `foreign_partitions` (`user/coreutils/src/bin/soso-install.rs:381-398`) hay que leer el nombre con `gptdisk::entry_name_ascii` y no empujar la entrada si el nombre es `SOSOINSTALL`, aunque `is_foreign` diga `"windows"` por el GUID `0700`. `is_foreign` se queda como está para un `T_MSDATA` con otro nombre, `ms-reserved` y `windows-recovery`. Hecho en host: el test junto a `etiquetas_y_ajenos` — `T_MSDATA` + nombre `SOSOINSTALL` no entra en la lista ajena; `T_MSDATA` con otro nombre y `T_MSRESERVED` sí. Hecho en placa: `soso-install nvme1` sobre un disco ya marcado `soso instalado` llega a la pregunta de borrar sin `--force` y sin exit 1. Esta copia ya está en nvme1; el criterio se ve en la próxima reinstalación.

2. ~~La barrera USB no puede devolver éxito cuando `SYNCHRONIZE CACHE(10)` falla.~~ **Descartado** (hallazgo 5): Linux `sd_sync_cache` da éxito en ese mismo caso, y soso ya lo replica.

3. **Punteros TX módulo `max_tfd_queue_size` (punto A).** En `iwl_trans.c`, `data_txq_write/read`, `mgmt_txq_write/read` y `cmd_write/read` cuentan módulo 256 en 22000 y 65536 en AX210, como `iwl_txq_inc_wrap`. TFD, cuerpo y tabla BC se indexan con `ptr & (n-1)`. El doorbell lleva el puntero completo. El puntero inicial del `SCD_QUEUE_CONFIG` no se recorta a 16. El espacio libre se calcula como `iwl_txq_space` y el reclaim convierte el índice de la respuesta a ranura igual que `iwl_txq_reclaim`. Hecho en host: en el hostcheck, 40 TX seguidos en la cola de 16 escriben en el doorbell 1…40, nunca 15→0, y no hay dos TFD pendientes en la misma ranura. Hecho en placa: `soso-update aplicar` conecta con `185.199.111.133:443`, y ninguna resolución DNS tras más de 16 tramas muestra `id=distinto`.

4. **Varios paquetes por RB en 22000 (punto B).** `drain_rx_gen2` recorre el RB por `offset` alineado a `FH_RSCSR_FRAME_ALIGN` hasta `FH_RSCSR_FRAME_INVALID`, y corta tras el primero solo en AX210 o posterior. Hecho en host: un RB con `TX_CMD` y `RX_MPDU` seguidos entrega los dos. Hecho en placa: `rb_multi` mayor que 0 en la línea `wifi: pérdidas` de SOSOLOG.

5. **Contador de secuencia en datos no QoS (punto C).** En `iwl_mvm_eth_to_80211`, un contador por estación `+= 0x10`, como `ieee80211_tx_h_sequence`. Hecho en host: dos tramas seguidas llevan `seq_ctrl` distintos y crecientes. Hecho en placa: una captura en monitor del canal 3 muestra SN crecientes desde `84:1b:77:e1:20:71`.

6. **Visibilidad.** `wifi_send` devuelve su error al `TxToken` (`device.rs:164`) y se cuenta. `tx_full_drop` y los desbordes del anillo RX de 8 y del EAPOL de 4 salen en el log con límite de frecuencia (no en `wifi status`: ver «Qué se ha implementado»). Sin esto, el próximo SOSOLOG tampoco dirá si fue cola, desautenticación o beacons. Hecho en placa: los contadores aparecen en `SOSOLOG` tras `aplicar`.

7. **Enlace caído y rekey (puntos D y E).** Un consumidor EAPOL persistente tras el 4-way que pase las tramas a `Supplicant::handle_group`. DEAUTH, DISASSOC y `MISSED_BEACONS` ponen `associated`/`authorized` a falso con una línea de log. `net/mod.rs` detecta el enlace caído, cierra sockets, reasocia y relanza el DHCP. Hecho en host: banco de `soso-wpa2` con group 1/2 → 2/2; una DEAUTH simulada deja `connected()` en falso. Hecho en placa: reiniciar el AP con soso conectado vuelve a dar `net: dhcp …` sin reiniciar soso, y una sesión de más de una hora (un rekey del AP) mantiene SSH.

8. **Registro.** Matriz `ga107-igpu` y `ax200-wifi`, y este informe. `arranques_consecutivos_ok` se queda en 0: llegar a sosh una vez no es aceptación.

Los cambios de driver que este log justifica son los de WiFi (3–7). Ninguno de GPU ni de ethernet.

## Qué se ha implementado (10 oct, misma sesión)

Todo lo de abajo está verificado en host. Nada está verificado en placa: los criterios «Hecho en placa» de cada punto siguen pendientes.

- **1.** `gptdisk::entry_is_foreign` (tipo + nombre). `soso-install` la usa en `foreign_partitions`. Test `sosoinstall_no_es_windows`.
- **3.** `txq_ptr_mask` (`0xff` en 22000/8000, `0xffff` en AX210) en `iwl_trans.c` para colas de datos, mgmt y comandos. La ranura sigue siendo `ptr & 15` (`& 31` en comandos). El doorbell lleva el puntero completo. El reclaim mide el avance contra lo que hay en vuelo. El índice TFD del 8000 va módulo 256. `ring_soak` comprueba ahora que lo que el hardware ve pendiente (doorbell menos su lectura) es lo que el driver tiene en vuelo. **Falla con el driver anterior en la trama 16** y pasa con el nuevo. `hcmd_queue` envía 300 HCMD y comprueba el doorbell.
- **4.** `iwl_trans_rx_rb`: recorre el RB como `iwl_pcie_rx_handle_rb` (alineado a 64, para en `0x55550000`, corta tras el primero en AX210). Lo usan el drenaje gen2 y el del 8000. Nuevo contador `rx_multi_rb`. Test: un RB de AX200 con ALIVE + MPDU entrega los dos; en AX211 solo el primero.
- **5.** `iwl_mvm_tx_8023` pone `seq_ctrl` desde `tx_seq_ctrl`, que avanza `+= 0x10` por trama encolada. El firmware no lo hace: la API TX gen2/gen3 no tiene `TX_CMD_FLG_SEQ_CTL`. Test: tres tramas llevan `0x0000`, `0x0010` y `0x0020`.
- **6.** El `TxToken` WiFi cuenta los rechazos de `wifi_send` (`tx_err`). Las colas de entrega RX (8) y EAPOL (4) ya no sobrescriben: con la cola llena, `head == tail` la hacía parecer vacía y se perdían de golpe todas las pendientes. Ahora se descarta la trama nueva y se cuenta (`rxq_full_drop`, `eapolq_full_drop`). Una línea `wifi: pérdidas …` sale en SOSOLOG cuando cambia algún contador, como mucho cada 10 s. **No va en `wifi status`:** `WifiStatus` es ABI de tamaño fijo, y con `flash --only kernel` un kernel nuevo escribiría fuera del búfer de un `sosh` viejo.
- **7.**
  - `iwl_mvm_link_down`: lo disparan una DEAUTH o DISASSOC del BSSID (dirigida o de difusión) y `MISSED_BEACONS_NOTIFICATION` con más de 16 consecutivos, el umbral de `iwl_mvm_connection_loss`. Pone `associated`, `authorized` y `keys_installed` a 0, registra causa y reason code, y marca el transporte para rearrancar el firmware en la próxima conexión.
  - El supplicant vive tras el 4-way (`wifi_wpa::SESION`), y `wifi_wpa::mantener()`, llamado desde el scheduler en la BSP, contesta la renovación de grupo y un M3 repetido.
  - Al caer el enlace, `net::on_wifi_lost` quita la IP, cierra los servicios y para el DHCP. La reconexión reutiliza SSID y PMK guardados, empieza a los 2 s y dobla la espera hasta 60 s. Al conseguirla se llama a `on_wifi_connected`, que relanza el DHCP.
  - En `soso-wpa2`, un M3 repetido o un mensaje de grupo repetido se contestan sin reinstalar la clave, como hace wpa_supplicant desde KRACK. Tests nuevos: M3 repetido, grupo repetido y renegociación de PTK. DEAUTH y beacons se prueban en `assoc_abi`.
- **Riesgo de 7 en placa:** la reconexión rearranca el firmware con `iwl_trans_recover` + `iwl_ax211_start_firmware`. Ese camino está probado en host (`hcmd_queue`, `check_recover_then_scan`) pero no hay registro de que haya funcionado en placa. Si falla, el log dice `recuperación falló` y soso se queda sin red, igual que antes de este cambio.

## Qué no se ha hecho

No se ha incrementado `arranques_consecutivos_ok`.

## Segunda sesión en placa (14:21, kernel y rootfs con los cambios de arriba)

- **Red:** asociación, 4-way y DHCP correctos. Unos 25 minutos sin caídas (`caídas=0`), ni panic ni excepción. Todas las respuestas DNS llegan con `id=ok`, sin ningún `id=distinto`. Las dos tandas de `soso-update comprobar` conectan con `185.199.109.133:443` y descargan el manifiesto entero (2,9 MB) por TLS: el síntoma del punto A ya no aparece. `rb_multi` llega a 38, así que el punto B pasaba en el AX200.
- **Pérdida nueva a la vista:** `rxq_llena` = 342 frente a `rx_ok` = 6272, y sube también en reposo. Cada `receive` de smoltcp vacía el anillo de 32 RB en la cola de entrega, que tenía 8 ranuras, y con DTIM 1 el AP suelta el tráfico de grupo en ráfaga tras cada beacon. La cola pasa a 64 ranuras (`IWL_RXQ_N`). Se descarta frenar el vaciado del anillo (contrapresión): retendría también las respuestas a comandos del firmware.
- **`soso-update: manifest no válido`:** no es la red. La release 0.3.8 (`666562e6f`) tiene `bin/rustc` de 222 MiB y un `.rmeta` de 65,5 MiB, por encima de `MAX_FILE_SIZE` (64 MiB), y 8 ficheros vacíos que `validate()` rechaza (`EmptyFile`). Corregido así:
  - `MAX_FILE_SIZE` = 512 MiB, porque `soso-update` sostiene el fichero en curso en un heap que acaba en `BRK_MAX`, 1,5 GiB;
  - `pack_rootfs` no empaqueta ficheros vacíos (`aplicar` nunca borra);
  - `xtask release` aborta si el manifiesto no pasa `validate()`.

  El manifiesto publicado, sin los vacíos, valida con el límite nuevo. Un soso instalado con el `soso-update` viejo seguirá rechazando cualquier release con `rustc` hasta que lleve el cliente nuevo: hay que reinstalarlo desde USB.

## Tercera sesión en placa (16:01, flush #54, 234 s)

- Arranque, WiFi, DHCP y TLS iguales que a las 14:21. DNS todo `id=ok`. Sin panic ni caídas. `dns github.com` resuelve. Dos `soso-update comprobar` conectan a `185.199.109.133:443` y vuelven a salir con `manifest no válido`: la release publicada sigue siendo la 0.3.8, y ningún cliente (viejo por `FileTooLarge`, nuevo por `EmptyFile`) la acepta. Hace falta publicar una release nueva, no más flashes del cliente.
- `rxq_llena` = 129 / `rx_ok` = 4854 (2,7 %). A las 14:21, con cola de 8, era 5,5 %. La de 64 ayuda y no basta: sigue desbordando en las ráfagas DTIM.
- `SOSOBOOT.TXT` ya dice `DONE Boot0001 soso` y «ya puedes quitar el USB»: el shim registró la entrada del NVMe. `SOSOUPD` y `SOSORES` siguen vacíos (no hubo aplicar).
