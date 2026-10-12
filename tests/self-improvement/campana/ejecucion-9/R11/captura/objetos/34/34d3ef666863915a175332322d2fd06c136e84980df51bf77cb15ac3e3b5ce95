# WiFi operativa para desarrollar desde soso

Investigación del 15 de septiembre de 2026. Código revisado: `dfec53f84`
con los cambios locales existentes. **La conexión WiFi completa sigue sin
estar validada en placa.** Los defectos de código que bloqueaban transporte,
datos y WPA2 están corregidos; superar ALIVE o detectar redes sigue sin
acreditar conectividad IP.

## Estado de implementación (15 sep 2026, tarde)

| Punto | Estado | Evidencia |
|---|---|---|
| 1. Formato DMA de recepción AX211 | **implementado** | `rx_datapath` hostcheck |
| 2. SCD en AX200 | **sin cambios**; sigue pendiente de placa | — |
| 3. Datos 802.11 ↔ Ethernet, incluido EAPOL | **implementado** | `rx_datapath` hostcheck |
| 4. Supplicant WPA2 y claves | **implementado** | `cargo test -p soso-wpa2` (27) |
| 5. DHCP, SSH, reconexión y Forja | **pendiente**: exige AP y placa | — |

Lo implementado son contratos de formato, conversión y protocolo comprobados
en el host. **Ningún banco de host acredita hardware**: los puntos 1 y 3
necesitan un arranque físico que llegue a ALIVE y a tráfico real, y el 4
necesita un AP que complete el 4-way. Los comandos de verificación están al
final.

## Resultado y prioridad

1. ~~Corregir el formato DMA de recepción AX211~~ y validar ALIVE real.
2. Validar en AX200 las correcciones de SCD ya presentes en el árbol.
3. ~~Completar el intercambio de datos 802.11 ↔ Ethernet, incluido EAPOL.~~
4. ~~Corregir el supplicant WPA2 y conectar las claves con el cifrado de datos.~~
5. Validar DHCP, SSH, reconexión y una compilación remota con Forja.

La AX200 del ROG es el banco más adelantado para asociación y tráfico. La
AX211 del host permite investigar el arranque gen3, pero actualmente sostiene
la conexión de Linux: pasarla a VFIO interrumpiría esa conexión. Esta sesión
ha usado lectura de registros, código y pruebas de host.

## Evidencia física: dos equipos

| Equipo | Evidencia disponible | Última etapa acreditada | Bloqueo observado |
|---|---|---|---|
| ROG, AX200 `8086:2723` | `target/usb-diagnostic-2026-09-15/SOSOLOG.TXT`, flush 27 | ALIVE, INIT_COMPLETE, MVM y scan de 20 BSS | `SCD_QUEUE_CONFIG grp=5 id=0x17 ver=3 tid=15` expira antes de AUTH |
| Host MSI, AX211 `8086:7f70` | Lectura actual de ESP, `target/wifi-investigation-2026-09-15/SOSOLOG.TXT`, flush 19 | PCI y carga de ucode/PNVM | `timeout ALIVE INT=0x80000003`, cuatro recepciones vacías, `alive=false` |

Ambos logs muestran `soso 0.2.2 (dfec53f84-dirty)`. Ese identificador no
permite distinguir todas las variantes locales del kernel. El número de
flush es del arranque, no una fecha ni un número de versión. La lectura
actual del USB no revalida las correcciones de SCD del ROG.

ESP `/dev/sda1` montada de solo lectura mediante udisks y desmontada después.
Se copiaron SOSOLOG y SOSODRV; no se grabó una imagen ni se reinició hardware.
SHA-256 de la copia AX211:

- SOSOLOG: `e6aa77f606e535885157cc33674d9dbb6f0655ab4b07642d731e65a8ef292d96`
- SOSODRV: `38419bec6cd2880cbd5ec2e503956df35f1ea4e89039783202f93e111416b2b9`

En Linux, `ethtool -i wlp128s20f3` confirma `iwlwifi`, firmware
`89.123cf747.0 so-a0-gf-a0-89.uc`, en `0000:80:14.3`. Es la misma familia
de firmware elegida por soso; el enlace funciona en Linux. La configuración
actual de la ESP no contiene SSID ni PSK efectivos. El log ROG muestra que
el autoconnect buscaba `soso-open`; las pruebas manuales usaron otra red.
Configurar credenciales será necesario, pero no explica los fallos anteriores.

## 1. AX211: el contrato DMA de RX es incorrecto

En [`iwl_trans.c`](../lxdde/ports/iwlwifi/iwl_trans.c), `iwl_alloc_queues()` y
`drain_rx_gen2()` usan estos formatos para `gen3`:

| Elemento | soso actual | Linux para AX210/SO (AX211) |
|---|---|---|
| Descriptor de buffer libre | Dirección de 8 bytes sin identificador | 16 bytes: `rbid` en offset 0, dirección en offset 8 |
| Descriptor completado | Entrada de 2 bytes | 32 bytes: `rbid` en offset 4 |
| Identidad del buffer | Índice directo, admite 0 | VID de 1 a N, buffer `VID - 1` |
| Descriptor de MPDU | Salta `sizeof(iwl_rx_mpdu_res_start)`, **4 bytes** | Descriptor de la API MQ para AX210, `sizeof(iwl_rx_mpdu_desc)` |

Referencias: Linux [descriptores](https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/wireless/intel/iwlwifi/pcie/internal.h),
[asignación, publicación y consumo de RX](https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/wireless/intel/iwlwifi/pcie/rx.c),
y el árbol local `mvm/ops.c`, `iwl_op_mode_mvm_start()`.

Una prueba contra el C actual publica un ALIVE en el buffer identificado por
VID=2. El consumidor AX200 lo encuentra; el consumidor AX211 no. Otra prueba
comprueba que el primer descriptor libre no contiene el formato AX211.
**La discrepancia está demostrada; que sea la única causa del timeout físico
todavía debe comprobarse.** No conviene aumentar el timeout antes de arreglarla.

**Implementado.** `iwl_internal.h` define ahora `struct iwl_rx_transfer_desc`
(16 B) y `struct iwl_rx_completion_desc` (32 B, `rbid` en el offset 4), con
asertos de tamaño y offset. `iwl_alloc_queues()` reserva `IWL_RX_BD_SIZE_GEN*`
y `IWL_RX_CD_SIZE_GEN*` por generación, y publica los BD con `iwl_rx_post_bd()`,
que escribe el formato que toca; `drain_rx_gen2()` lee el VID con
`iwl_rx_completed_vid()`, valida el rango 1..N y cuenta en `rx_vid_drop` lo que
no designa buffer, en vez de reciclar un índice inventado. El descriptor de
MPDU pasa a `IWL_RX_DESC_SIZE_V3` (56 B) en gen3, con `struct
iwl_rx_mpdu_desc_v3` y accesorios `v3` para canal y energía.

Queda el **arranque físico**: ALIVE + INIT_COMPLETE + scan completo en la
AX211. Si sigue fallando, capturar descriptores y registros antes del reciclaje
y revisar IML/PNVM y errores de firmware.

## 2. AX200: SCD sigue pendiente de verificación física

El árbol ya contiene cambios de cabecera HCMD `version=0`, primer bloque DMA
separado, TID de gestión 15 y LQ asíncrono. La cabecera de comando y la versión
del payload son conceptos distintos: Linux usa `WIDE_ID(...)` sin versión
explícita en este comando y elige por separado el payload SCD v3.
Referencias: [HCMD gen2](https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/wireless/intel/iwlwifi/pcie/tx-gen2.c)
y [asignación de TXQ](https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/wireless/intel/iwlwifi/queue/tx.c).

El hostcheck actual pasa, incluida la cabecera SCD. No demuestra que el
firmware haya aceptado la cola. El siguiente arranque debe mostrar
`SCD wide ver_hdr=0 ver_tlv=3`, un `TXQ mgmt qid=...` válido y respuestas
AUTH/ASSOC del AP. Ver [diagnóstico ROG](DIAGNOSTICO-ROG-2026-09-15.md).

El mensaje `LQ_CMD ... async` acredita encolado, no ejecución en firmware.
Tampoco demuestra que LQ nunca produzca respuesta: el transporte mantiene
slots asíncronos hasta recibirla. Si SCD sigue sin contestar, inspeccionar
también el LQ precedente, sus capacidades y el error del firmware.

## 3. Falta el camino de datos, incluso para una red abierta

### Recepción

`parse_rx_mpdu()` entrega las tramas solamente al scan y al manejador de
AUTH/ASSOC. No desencapsula datos ni los encola para `wifi_receive()`.
La única entrega de Ethernet desde el transporte está condicionada a
`group=5, cmd=1`. Ese identificador es **UPDATE_MU_GROUPS_CMD**, no una
notificación de Ethernet, en la [API Intel](https://raw.githubusercontent.com/torvalds/linux/v6.6/drivers/net/wireless/intel/iwlwifi/fw/api/datapath.h).

**Implementado.** `parse_rx_mpdu()` entrega ahora las tramas de datos a
`iwl_mvm_rx_to_eth()` ([`iwl_mvm_data.c`](../lxdde/ports/iwlwifi/iwl_mvm_data.c)),
que valida tipo y subtipo, exige FromDS sin ToDS, calcula la cabecera real con
`iwl_80211_hdrlen()` (QoS, HT Control, cuatro direcciones), comprueba BSSID y
destino, salta los 8 B de cabecera CCMP en las tramas protegidas y desencapsula
LLC/SNAP (RFC 1042 y bridge tunnel) antes de armar la trama Ethernet con
`da = addr1` y `sa = addr3`. El resultado se reparte: EAPOL a la cola del
supplicant y el resto a la pila IP.

La entrega por `group=5, cmd=1` está **eliminada**: `UPDATE_MU_GROUPS_CMD` no
es una notificación de Ethernet y su cuerpo entraba en smoltcp como un paquete
inventado.

El descifrado se comprueba con `iwl_rx_crypto_ok()` sobre el DW5 del descriptor:
exige `SEC_CCM`, `DECRYPTED` y `MIC_OK`. Con claves ya instaladas, una trama de
datos **sin** proteger se descarta; antes de instalarlas se acepta, que es lo
que permite que pase el propio 4-way. El PN lo valida el firmware, que es quien
recibe la RSC en `ADD_STA_KEY`.

### Transmisión

[`iwl_mvm_tx_8023()`](../lxdde/ports/iwlwifi/iwl_mvm.c) actualmente:

- Pone `frame[1]=0x02` (FromDS) para enviar desde una estación; corresponde ToDS.
- Copia la trama Ethernet completa detrás del encabezado 802.11, sin LLC/SNAP.
- Fija addr3 al BSSID, perdiendo el destino Ethernet real.
- Rechaza tramas mayores de 450 bytes mientras smoltcp anuncia 1514.
- Activa `IWL_TX_FLAGS_ENCRYPT_DIS` siempre, sin cabecera CCMP ni PN de datos.

**Implementado.** `iwl_mvm_eth_to_80211()` construye la MPDU ToDS con
`addr1 = BSSID`, `addr2 = estación` y `addr3 = destino Ethernet real`, y
encapsula la carga en LLC/SNAP en vez de copiar la trama 802.3 entera.
`IWL_MGMT_TX_SLOT_SIZE` sube a 2048 B para que quepa una trama de MTU, y el
límite de 450 bytes pasa a `IWL_MAX_ETH_FRAME` (1514). `IWL_TX_FLAGS_ENCRYPT_DIS`
sólo se pone mientras no haya claves, como en `iwl_mvm_set_tx_cmd`: con la API
nueva de TX el firmware inserta la cabecera CCMP y el PN, así que el driver no
reserva hueco ni marca Protected.

La ocupación del anillo la llevan `iwl_trans_tx_space()` y
`iwl_trans_tx_reclaim()`: `parse_tx_resp()` avanza el consumidor con el índice
de la secuencia, `iwl_trans_tx()` rechaza cuando no queda hueco y lo cuenta en
`tx_full_drop`. Una respuesta repetida no retrocede el consumidor.

El banco cubre EAPOL, IP, unicast, difusión, otra BSSID, otra estación, tramas
protegidas con MIC bueno y malo, tramas en claro con y sin claves, el MTU
completo y el agotamiento del anillo. **Quedan las pruebas contra un AP real**:
ARP, DHCP y transferencia sostenida.

## 4. WPA2: fallos reproducidos en el supplicant actual

Se ejecutó una copia del módulo real
[`wifi_wpa.rs`](../kernel/src/net/wifi_wpa.rs) en host, sustituyendo únicamente
VFS, reloj y transporte por dobles de prueba. No se alteraron sus funciones.

| Prueba | Resultado | Consecuencia |
|---|---|---|
| PBKDF2 `password` / `IEEE` | Pasa | La derivación inicial PMK funciona |
| M1 válido del AP, `key_info=0x008a` | Falla: cero envíos | Se descarta todo M1 con ACK activado |
| EAPOL de 4 bytes | Falla: panic por índice 5 | Lee campos antes de validar longitud suficiente |
| PTK contra vector PRF independiente | Falla | Falta contador en el primer bloque; genera 40 de 48 bytes |
| Cabecera M2 | `0x0388`, esperado `0x010a` | Flags, versión y posiciones de escritura incorrectos |
| Cabecera M4 | `0x0388`, esperado `0x030a` | Mismo defecto de codificación |
| Extracción GTK KDE | Desplazada un byte | Incluye el reservado y pierde el último byte de la clave |

El formato de los campos y su orden de bytes están definidos en
[wpa_common.h](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/master/src/common/wpa_common.h).
La PRF incluye el contador desde cero en **cada** bloque y genera la longitud
solicitada: [sha1-prf.c](https://android.googlesource.com/platform/external/wpa_supplicant_8/+/refs/heads/main/src/crypto/sha1-prf.c).

Otros defectos visibles en código que deben cubrir las siguientes pruebas:

- M2 no incorpora el RSN IE negociado; M4 conserva campos y longitud de M3.
- Se instala `ptk[0..16]` (KCK) como clave de datos; CCMP necesita la TK,
  `ptk[32..48]` en este perfil.
- Se busca GTK directamente en Key Data sin desenvolver AES con la KEK.
- Se ignoran errores al enviar M4 e instalar claves; una GTK inválida se omite.
- No se valida suficientemente replay counter, ANonce ni procedencia del AP;
  faltan retransmisiones y manejo de renovación de claves.

La máquina de estados de referencia acepta mensajes del AP con ACK, valida
el MIC y descifra Key Data antes de instalar claves:
[wpa_supplicant](https://chromium.googlesource.com/external/w1.fi/cgit/hostap/+/refs/heads/pending/src/rsn_supp/wpa.c).

`iwl_mvm_install_key()` tampoco distingue adecuadamente claves por pares y
de grupo: el índice GTK está fijado por el llamante, falta el flag multicast
y no se transmite el contador recibido. Añadir TK/GTK al firmware por sí
solo no habilita el TX protegido descrito arriba. Referencia local:
`mvm/sta.c`, `mvm/tx.c` y `fw/api/sta.h` de Linux.

**Implementado.** La máquina WPA2-PSK/CCMP vive ahora en el crate `no_std`
[`soso-wpa2`](../crates/soso-wpa2), y `kernel/src/net/wifi_wpa.rs` se queda
sólo con la E/S. Lo corregido, punto por punto:

- La PRF lleva el contador **desde el primer bloque** y genera la longitud
  pedida (`crypto::prf_sha1`): antes faltaban 8 de los 48 bytes de la PTK.
- Los mensajes del AP se aceptan **por** llevar ACK, no se descartan por ello.
- `EapolKey::parse()` valida longitudes y tipos antes de leer ningún campo: una
  carga de 4 bytes devuelve `NotEapol(TooShort)`, no un pánico por índice.
- M2 codifica `0x010a` y M4 `0x030a` en las posiciones correctas, con la
  longitud del cuerpo recalculada y `key_length = 0` como en RSN.
- M2 repite el RSN IE que el driver anunció en la Association Request
  (`iwl_mvm_rsn_ie()`, la misma función que construye el IE del ASSOC).
- M4 se construye de cero, no reutiliza los campos ni la longitud de M3.
- Key Data se **desenvuelve con AES-KEK** (RFC 3394) cuando el AP pone
  `ENCR_KEY_DATA`, con control de integridad.
- El KDE de la GTK se lee en el offset correcto (i+8): antes empezaba en el
  byte reservado y perdía el último byte de la clave.
- Se instala la **TK** (`ptk[32..48]`), no la KCK, y la GTK va por
  `iwl_mvm_install_gtk()` con `STA_KEY_MULTICAST`, su Key ID del KDE, ranura de
  firmware propia y la Key RSC del mensaje.
- Se validan contador de reenvío, ANonce de M3 contra la de M1 y MIC antes de
  descifrar nada. Un M3 sin GTK utilizable **no autoriza** el enlace.
- Cada descarte tiene motivo (`Discard`) y se registra: el silencio era lo que
  dejaba el handshake colgado sin decir dónde.
- Hay renovación de clave de grupo (mensaje 1 de 2 → GTK nueva + respuesta).
- Los errores de envío de M4 y de instalación de claves ya no se ignoran.

`asociada` y `autorizada` son ahora estados distintos: `iwl_ax211_authorized()`
sólo vale 1 en red abierta tras asociar, o en WPA2 tras el 4-way.
`net::wifi_link_up()` mira **autorizada**, y `WIFI_FLAG_AUTHORIZED` lo expone a
`sosh wifi status`. EAPOL tiene su propia cola (`iwl_ax211_rx_eapol`), así que
smoltcp no puede llevarse M1/M3 durante la autenticación.

**Queda**: un AP real que complete el 4-way, y la limpieza de claves, colas y
estado IP en la reconexión.

## 5. Criterios de cierre y relación con Forja

| Hito | Prueba de aceptación |
|---|---|
| Firmware | ALIVE real, INIT_COMPLETE, MVM listo y scan terminado por UID |
| Asociación | TX completado y respuestas AUTH/ASSOC válidas con AID |
| Datos | ARP + DHCP real en un AP de laboratorio; tramas de tamaño MTU en ambos sentidos |
| WPA2 | M1–M4 válidos, TK/GTK instaladas, tráfico CCMP bidireccional y errores rechazados |
| IP y aplicaciones | Lease, ruta, DNS, SSH por puerto 22 y descarga HTTPS verificable |
| Estabilidad | Tres arranques, sesión con transferencia sostenida, caída del AP y reconexión con lease nuevo |
| Desarrollo desde soso | Editar `hola-std`, sincronizar por WiFi, compilar con Forja, aplicar el ELF y ejecutarlo |

La integración DHCP ya existe (`net::on_wifi_connected()`); en WiFi no hay
fallback a 10.0.2.15. Debe activarse tras autorización WPA2 y recibir paquetes
desde el driver. La IP de Forja será la del host en la LAN; `10.0.2.2` es
la dirección del host en las pruebas QEMU con slirp.

El primer cierre útil es el bucle de compilación remota documentado en
[SELF-HOSTING.md](SELF-HOSTING.md). El compilador Rust nativo completo sigue
pendiente; `soso-forja local` planifica y `build-local` copia artefactos ya
construidos. Tener WiFi permite usar el camino remoto existente.

## Verificación realizada y límites

Las sondas desechables de la investigación son ya pruebas permanentes contra el
código de producción; `target/wifi-investigation-2026-09-15/` queda como
registro histórico y **no** hay que reejecutarlo: su copia del supplicant es
una instantánea del módulo viejo y sus fallos ya no describen el árbol.

```sh
./scripts/l6-iwl-fw-hostcheck.sh     # rx_datapath + ring_soak, con ASan+UBSan
cargo test -p soso-wpa2              # supplicant: vectores y transcripciones
cargo xtask check                    # lo anterior + builds bare-metal + suite host

sudo ./scripts/l6-wifi-capture-4way.sh   # una sola vez: captura el 4-way real
```

Estado el 15 de septiembre de 2026 por la tarde:

- `./scripts/l6-iwl-fw-hostcheck.sh`: **pasa**, incluidos ASan y UBSan. Dos
  bancos nuevos:
  - `rx_datapath` (36 comprobaciones): descriptores RX de las dos generaciones,
    conversión en los dos sentidos y ocupación del anillo TX, con tramas
    armadas a mano.
  - `ring_soak`: los anillos RX y TX contra un **modelo de firmware** que
    consume el anillo con el mismo formato que escribe el driver y lleva su
    propia contabilidad. Cuatro mil vueltas por anillo y generación, con
    invariantes de propiedad (ningún buffer publicado dos veces ni reciclado
    antes de tiempo), de conservación (los 32 buffers siempre repartidos) y de
    orden (ningún paquete perdido, repetido ni desordenado); en TX, que nunca
    se escriba en un TFD sin confirmar y que el hueco declarado cuadre.
    Semilla fija y reproducible (`SOSO_IWL_SOAK_SEED`, `SOSO_IWL_SOAK_ITERS`).

  El modelo de firmware **no es el hardware**: sale de la misma lectura de la
  especificación que el driver, así que no puede descubrir que el formato sea
  otro. Lo que sí encuentra es que las dos mitades del driver se contradigan
  bajo carga. Se comprobó inyectando cuatro defectos —el formato de BD de gen2
  en gen3, la cola TX sin control de ocupación, el reciclado en la ranura
  equivocada y el consumidor de TX sin la guarda de no retroceder— y el banco
  caza los cuatro.
- `cargo test -p soso-wpa2`: **27 pasan**. Dentro hay dos vectores publicados
  (PBKDF2 de IEEE 802.11i y AES Key Wrap del RFC 3394) y, para la PRF, un
  contraste con una segunda implementación escrita desde la norma más un AP
  simulado que deriva su PTK por ese otro camino. Eso acredita que los dos
  lados coinciden y fija los defectos concretos; **no** es una captura real.
- `transcripcion_real`: **se salta** mientras no exista
  `crates/soso-wpa2/tests/fixtures/4way-hostapd.txt`. Con el fixture puesto,
  alimenta el M1 y el M3 de hostapd y exige que **nuestro M2 y nuestro M4
  salgan byte a byte iguales a los de wpa_supplicant, MIC incluido**: eso
  valida de una vez PBKDF2, la PRF, la derivación de la PTK, la codificación de
  `key_info`, las longitudes y el HMAC contra implementaciones ajenas. El
  fixture lo genera `sudo ./scripts/l6-wifi-capture-4way.sh`, que carga
  `mac80211_hwsim` con dos radios virtuales, levanta hostapd y wpa_supplicant,
  captura sólo las tramas EAPOL y descarga el módulo al salir — **no toca el
  WiFi real del equipo**. Necesita `hostapd` instalado.
  Como un test saltado sale «ok», `cargo xtask check` avisa explícitamente
  cuando la captura falta.
- `cargo xtask test` (QEMU): pasa salvo `voz`, que es flaky por un pánico de
  kernel previo y no toca WiFi. QEMU no lleva iwlwifi: **no acredita nada** de
  esta ruta.
- Matriz AX211: ALIVE sigue en **fail**. El arreglo de descriptores no cambia
  la matriz hasta que haya un SOSOLOG nuevo de placa.

Lo que sigue **sin acreditar**, y con qué se acreditaría:

| Hito | Qué falta |
|---|---|
| ALIVE real en AX211 | `sudo ./scripts/l6-wifi-vfio-test.sh`, o arranque live; luego `cargo xtask hw-matrix parse-logs --id ax211-wifi` |
| SCD en AX200 | Arranque en el ROG: `SCD wide ver_hdr=0 ver_tlv=3`, `TXQ mgmt qid=…` y respuestas AUTH/ASSOC |
| 4-way contra implementación ajena | `sudo ./scripts/l6-wifi-capture-4way.sh` (hwsim, sin hardware) |
| Datos y 4-way en el aire | AP de laboratorio: ARP, DHCP, tráfico CCMP en los dos sentidos |
| IP y aplicaciones | Lease, ruta, DNS, SSH en el 22 y descarga HTTPS |
| Desarrollo desde soso | El bucle de [SELF-HOSTING.md](SELF-HOSTING.md) sobre WiFi |

Un hostcheck verde no sustituye la prueba del AP físico.
