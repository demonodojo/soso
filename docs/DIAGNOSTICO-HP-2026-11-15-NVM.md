# Diagnóstico USB — HP (iGPU Intel + 8265) — 2026-11-15 — flush #26

Arranque real desde el USB live. El kernel llega a `sosh` y al escritorio, pero la
WiFi 8265 **no pasa de ALIVE**: `iwl_mvm_nvm_read_8000` rechaza la NVM y la
calibración del INIT ucode falla antes de `TX_ANT`/`PHY_CFG`. Este informe fija
la evidencia, la compara con Linux 6.6.32 y deja el orden de corrección.

## 1. Evidencia y alcance

- **ESP**: `/dev/sda1` (vfat, LABEL `KERNEL`, 192 MiB), montada con `udisksctl`
  y desmontada tras copiar. Particiones vistas: `sda1` ESP, `sda2` 6,6 G,
  `sda3` 493 G, `sda4` `SOSOINSTALL` 64 M (no se tocó).
- **Copias**: `target/usb-diagnostic-2026-11-15-run26/` — `SOSOLOG.TXT` (original
  de 256 KiB) y `SOSOLOG.txt` (recorte de 424 líneas con `tr -d '\000'`),
  `SOSODRV.txt`, `SOSOHASH/BOOT/OBOOT/UPD.TXT`, `SOSOKRN.MET` (512 B),
  `hostcheck-iwl.log`, `hostcheck-gsp.log`. Las rutas se citan como `L###` en
  este documento.
- **Identidad**: `soso 0.3.28 (e466e0965)` (L50) = HEAD del repo, sin `-dirty`.
  `SOSOHASH.TXT` sigue diciendo `build=886e8bb0d-dirty`: `--only kernel`
  reescribe `KERNEL~1`/`SOSOKRN` pero **no** `SOSOHASH`
  (`xtask/src/flash_usb_live.rs:695-707` vs `xtask/src/package_live.rs:680-741`),
  así que el desfase es esperado y la identidad buena es el banner del kernel.
- **Equipo**: HP. iGPU Intel `8086:5917` (**no hay GPU NVIDIA**: ninguna etapa
  `gpu` aplica), WiFi `8086:24fd` (Dual Band Wireless-AC 8265, familia 8000),
  ethernet `10ec:8168` rev 15, nvme `1c5c:1527`. `hwscan: 23 dispositivos PCI,
  5 con driver conocido` (L388); `SOSODRV` trae 20 `drv:` + 2 `usb:`.
- **Árbol Linux**: `lxdde/linux` 6.6.32 (extraído por `cargo xtask lx-build`, sin
  `.git`; no hay tag que anotar). Rutas citadas relativas a
  `drivers/net/wireless/intel/iwlwifi/` y `drivers/usb/host/`.
- **Hostchecks** (baratos, no validan silicio): `scripts/l6-iwl-fw-hostcheck.sh`
  exit 0, `scripts/l6-g3-gsp-hostcheck.sh` exit 0 (GSP sin GPU NVIDIA: solo
  protocolo). Salidas en `hostcheck-iwl.log` / `hostcheck-gsp.log`.
- **No se valida aquí**: el contenido real del OTP/NVM de esta placa, la causa
  exacta del rechazo (no hay traza, ver H1/H2) ni el canal RF.

## 2. Tabla de etapas

| Etapa | Cita del SOSOLOG | Resultado |
|---|---|---|
| Kernel + userspace | `soso 0.3.28 (e466e0965)` L50; `sosh — escribe 'help'` L416 | OK |
| Escritorio | `tcp: listen puerto 7432 slot 0 smoltcp=ok` L423; `7430 slot 3` L424 | OK (WM arriba) |
| hwscan | `hwscan: 23 dispositivos PCI, 5 con driver conocido` L388 | OK (sin NVIDIA) |
| xHCI init | `initialization complete, 18 ports available` L164 | OK |
| xHCI mass storage | `slot=1 capacidad 1048576000 sectores` L187; `SYNCHRONIZE CACHE(10) no soportado` L190 | OK (sync_cache=false) |
| xHCI puerto 6 | `command completion huérfano (ptr=0x0 … slot=2)` L196; `timeout after 5000 ms` L197; `Enable Slot falló en port 6 (1 seguidos)` L216 | **FAIL** (dispositivo sin enumerar, sin reintento) |
| WiFi probe | `probe id=0x24fd familia=8000 … bdf=0x200` L287 | OK |
| Firmware INIT | `firmware ALIVE (UCODE_ALIVE_NTFY)` L306; `INIT ucode ALIVE ok` L307; `fw_alive 8000 SCD=0x00817080 qid=0` L308 | OK |
| NVM 8000 | `send_cmd … grp=0 id=0x88` L308 → `iwl_mvm: NVM 8000 inválida o lectura fallida` L309 | **FAIL** |
| `init_complete` | `INIT 8000 calibración falló` L310 (nunca se enviaron `TX_ANT`/`PHY_CFG`) | **FAIL** |
| MVM | `lxdde: iwlwifi start rc=-1 phase=fw_start alive=true` L312 | **FAIL** |
| Recuperación | `recuperación falló (…, 1/2/3 de 3)` L348, L367, L385 — mismas 3 etapas | **FAIL** (3/3) |
| Scan | `wifi: escaneando redes…` L419; `error de E/S` L420 | **FAIL** |
| Assoc WPA2 | `wifi-wpa: fallo AUTH+ASSOC 802.11` L386 | **FAIL** |
| Ethernet | `net: backend rtl8169` L327; `net: dhcp…` L328 | **FAIL** (sin cable: `phystatus 0x84 bmsr 0x7989`, enlace DOWN 10M half) |
| Sesión | `sosh` + `soso-wm` con 7430/7432 | OK sin red; `wifi: sondeo parado 7 veces en 30000 ms (max 8857 ms, causa=volcado del log ~1722 ms)` L421 → degradada |

## 3. Hallazgos

### H1. La NVM llega, se responde y soso la rechaza sin decir por qué — confirmado (causa indeterminada)

- Síntoma: entre L308 y L309 no hay `iwl_trans: timeout cmd …` ni
  `iwl_rx: … sin emparejar` ni ningún recorte: el `NVM_ACCESS_CMD` obtuvo
  respuesta, así que el fallo está en una **validación de datos** dentro de la
  función, no en el transporte. La línea de éxito
  (`iwl_mvm: NVM 8000 versión=0x%x secciones=0x%x LAR=%u`) **no aparece**: nunca
  se alcanzó el final.
- soso: `lxdde/ports/iwlwifi/iwl_mvm_nvm.c:114-218`. Todos los caminos hacen
  `goto out` sin traza (`:139`, `:141`, `:151`, `:162`, `:169`, `:178`,
  `:186`, `:193`, `:201`) y solo se imprime el mensaje genérico en `:215`.
- Linux: `mvm/nvm.c:186-210` (`iwl_nvm_read_section`) y `mvm/fw.c:730`
  (`IWL_ERR(mvm, "Failed to read NVM: %d\n", ret)`) — el port no tiene
  equivalente de ese `-EINVAL`/`-ENODATA` con número de sección y offset.
- Conclusión: **confirmado** que el rechazo es de validación propia;
  **indeterminado** cuál de las nueve salidas se tomó. Es el primer hueco a
  cerrar: sin esa traza el resto del diagnóstico son hipótesis.

### H2. soso tira toda la NVM al primer error de sección; Linux sigue — confirmado (hipótesis causal)

- soso: cualquier chunk que no sea `status 0/1` aborta la lectura completa
  (`iwl_mvm_nvm.c:151`), y cualquier umbral de longitud propio también
  (`:162` sección 1 < 10 B, `:169` sección 3 < `2*51` o < `laroff+2`, `:178`
  sección 12 < 8 B).
- Linux: `mvm/nvm.c:317-326` — `if (ret == -ENODATA) { continue; }` y
  `if (ret < 0) break;`: un error de sección **no** invalida la NVM; después
  solo se exige lo obligatorio (`mvm/nvm.c:236-256`: SW + REGULATORY +
  `PHY_SKU` en B0, más `HW` **o** `MAC_OVERRIDE`).
- Por qué encaja con el síntoma: la lectura empieza por la **sección 0** (L129
  `for (section = 0; …)`), que Linux lee y tolera pero nunca usa; si el FW
  contesta `status` distinto de 0/1 en cualquier sección (0 incluida), soso
  abandona en el primer comando — exactamente lo que muestra L308→L309, con las
  3 recuperaciones fallando en el mismo punto.
- Estado: **confirmado** como discrepancia código-vs-código; **hipótesis** como
  causa de este arranque (no hay log que lo pruebe). Se cierra con H1.

### H3. Chunks de 256 B frente a 2048 B de Linux — confirmado (no causal por sí solo)

- soso: `cmd.length = 256` (`iwl_mvm_nvm.c:136`) y `cmd_resp[512]`
  (`lxdde/ports/iwlwifi/iwl_ax211.h:254`), con el comentario de `:112-113`
  («para que la respuesta quepa en cmd_resp»).
- Linux: `IWL_NVM_DEFAULT_CHUNK_SIZE (2 * 1024)` (`mvm/nvm.c:19`) y
  `while (ret == length)` (`:192`), con 2 s de timeout por comando
  (`mvm/nvm.c:161`).
- Efecto: 8× más HCMD por sección y, con H7, 8× más probabilidad de que un
  volcado del log se coma un timeout de 2000 ms (`iwl_mvm_nvm.c:137`).
- Estado: **confirmado**; **descartado** como causa única (hubo respuesta, no
  timeout). Va con H2 porque ambas se arreglan en la misma función.

### H4. MAC, radio, LAR, canales y límite de 32 KiB: iguales que Linux — descartado

Comparado línea a línea y **no** es la causa:

- MAC desde `MAC_OVERRIDE` (sección 11) en offset de byte 2, si no desde PRPH
  `0xA03080`/`0xA03084` con el mismo *flip*: soso `iwl_mvm_nvm.c:186-205` ≡
  Linux `iwl-nvm-parse.c:1248-1296` (`MAC_ADDRESS_OVERRIDE_EXT_NVM = 1` y
  `WFMP_MAC_ADDR_0 = 0xA03080` en `iwl-prph.h:344-345`). El MAC reservado
  `02:cc:aa:ff:ee:00` también coincide (`iwl-nvm-parse.c:1252-1256`).
- Antenas: `radio >> 24 & 15` / `>> 28 & 15` (`iwl_mvm_nvm.c:203-204`) ≡
  `iwl-nvm-parse.c:1207-1208` con `radio_cfg` en la sección 12 offset 0.
- LAR: `0x4C7`/`0x507` según versión y máscara `0x7` (`iwl_mvm_nvm.c:167-173`) ≡
  `iwl-nvm-parse.c:62-65` y `:1491-1499`.
- Canales: 51 entradas de 2 B desde el inicio de la sección 3
  (`iwl_mvm_nvm.c:170-172`) ≡ `IWL_NUM_CHANNELS` + `NVM_CHANNELS_EXTENDED = 0`.
- Límite de 32 KiB (`iwl_mvm_nvm.c:117`) ≡ `cfg/8000.c`
  (`OTP_LOW_IMAGE_SIZE_32K`) y `eeprom_size` de Linux.

### H5. Parseo RX gen1: idéntico a Linux — descartado

- soso: `lxdde/ports/iwlwifi/iwl_trans.c:1374` (`data = buf + 8`,
  `pay = len - 4`) y `:2078` (`iwl_trans_rx_rb`).
- Linux: `iwl-trans.h` — `struct iwl_rx_packet { __le32 len_n_flags; struct
  iwl_cmd_header hdr; u8 data[]; }` y
  `iwl_rx_packet_payload_len(pkt) = len - sizeof(pkt->hdr)` = `len - 4`, con
  `data` a 8 B. Es la misma aritmética para gen1 y gen2.
- Además el log no tiene ni un `iwl_rx:` (ni truncados ni huérfanos). Descartado.

### H6. xHCI pierde el `Enable Slot` del puerto 6 y no lo reintenta — confirmado (causa hipótesis)

- Síntoma: el puerto 6 está conectado y resetea bien (`port 6 reset complete:
  enabled=true speed=3` L194), pero la completion del `Enable Slot` llega con
  **`ptr=0x0`** y `code=1 (Success) slot=2` (L196). El driver la descarta como
  huérfana, espera 5 s (L197) y abandona el puerto (L216); el resto del escaneo
  sigue y ese dispositivo nunca se enumera.
- soso: `crates/xhci-nostd/src/driver.rs:765-772` descarta **cualquier**
  completion cuyo `TRB Pointer` no sea el del comando en curso; `:800` es el
  timeout; `:628-645` no reintenta el puerto (solo corta a los 2 fallos
  seguidos). El anillo es **serializado** (un único comando en vuelo), así que
  el descarte por puntero es más estricto que la situación real.
- Linux: `xhci-ring.c:1649` → `xhci_abort_cmd_ring()` (`:428`) al expirar un
  comando, con lista de comandos en vuelo (`xhci->cmd_list`) para casar cada
  completion; nunca deja el puerto sin un segundo intento.
- Descartada la hipótesis anterior («anillo de eventos con basura del firmware»):
  las DMA del port se piden con `alloc_zeroed_uc`
  (`kernel/src/drivers/usb_storage.rs:79-84`, `kernel/src/lxdde/dma.rs:14`), y
  los anillos van alineados a página (L133-134).
- Estado: **confirmado** el síntoma y el `ptr=0`; **hipótesis** por qué el
  controlador lo escribe a 0. Necesita traza del evento crudo antes de tocar
  nada más.

### H7. El volcado del log para el sondeo de WiFi hasta 8,8 s — confirmado (no causal aquí)

- Log: `wifi: sondeo parado 2772 ms (wifi::poll, pid=0, causa=volcado del log
  ~5652 ms)` L417 y `7 veces en 30000 ms (max 8857 ms)` L421, con
  `flush #26 … dur=16ms max=8864ms` L1.
- soso: `kernel/src/lxdde/wifi.rs:151-160` atribuye la parada a
  `fatlog::ultimo_volcado_ms()`; el volcado es síncrono
  (`kernel/src/drivers/fatlog.rs:435` `flush_inner_sync`, `:89`
  `flush_or_warn`) y escribe por BOT sobre el propio stick xHCI.
- Incompatible con los timeouts de 2 s de HCMD (`iwl_mvm_nvm.c:137`) y con las
  esperas de `INIT_COMPLETE` que vienen después de arreglar H2. En este arranque
  **no** causó el fallo (no hubo timeout de comando), pero hay que acotarlo
  antes de fiarse de cualquier timeout de la WiFi.

## 4. Orden de corrección

Bloqueantes primero. Todos son cambios de código del port; ninguno está hecho.

### 1. Trazar la NVM por sección y offset (`iwl_mvm_nvm.c`)

- Archivos: `lxdde/ports/iwlwifi/iwl_mvm_nvm.c` (una línea por intento antes de
  cada `goto out`, con sección, offset, `status`, `length`, `roff` y
  `cmd_resp_len`; y una sola línea final de éxito/fallo).
- Linux: `mvm/nvm.c:186-210` + `mvm/fw.c:730` (`Failed to read NVM: %d`).
- Hecho en host: caso nuevo en `tools/iwl-hostcheck/nvm_8000_test.c` donde la
  sección 0 contesta `status=2`; el banco actual solo cubre truncado, offset
  incorrecto y PHY_SKU ausente (`nvm_8000_test.c:51-70`).
- Hecho en placa: el siguiente SOSOLOG dice **qué** sección/offset/status
  rechazó, sin cambiar el comportamiento todavía.

### 2. Un error de sección no invalida la NVM (paridad con `iwl_nvm_init`)

- Archivos: `lxdde/ports/iwlwifi/iwl_mvm_nvm.c:129-190` (sustituir los `goto out`
  por sección por «marcar sección ausente y seguir», y dejar la obligatoriedad
  solo en la comprobación final de `present`: 1, 3, 12 y (10 **o** 11), que ya
  existe en `:183-186`).
- Linux: `mvm/nvm.c:317-326` (`-ENODATA → continue`, `< 0 → break`) y
  `:236-256` (solo lo obligatorio).
- Hecho en host: caso «sección 3 falla a mitad» → `nvm_ready=1` con las demás
  secciones; y caso «falta la 12» → sigue fallando. `cargo xtask check`.
- Hecho en placa: aparece la línea de éxito de `iwl_mvm_nvm_read_8000`
  (`iwl_mvm_nvm.c:210`) con `secciones=0x…` y el INIT sigue a `TX_ANT`/`PHY_CFG`.

### 3. Chunks de 2048 B y `cmd_resp` de 2056 (paridad de tamaño)

- Archivos: `lxdde/ports/iwlwifi/iwl_mvm_nvm.c:136` (`cmd.length`),
  `lxdde/ports/iwlwifi/iwl_ax211.h:254` (`cmd_resp[512]` → ≥ 2056) y el copiado
  de respuesta que use `cmd_resp_wire_len`/`cmd_resp_trunc` (`iwl_ax211.h:255-258`).
- Linux: `mvm/nvm.c:19` (`IWL_NVM_DEFAULT_CHUNK_SIZE`) y `:187-205`.
- Hecho en host: `nvm_8000_test.c` comprueba `cmd->length == 2048` y una sección
  de 4096 B gastando exactamente 2 comandos, sin `cmd_resp_trunc`.
- Hecho en placa: la misma NVM con 1/8 de HCMD y sin recortes.

### 4. xHCI: aceptar la completion del comando en vuelo y reintentar el puerto

- Archivos: `crates/xhci-nostd/src/driver.rs:765-772` (si el puntero no casa pero
  el anillo está serializado y el comando sigue en vuelo, aceptarla y avisar),
  `:800` (registrar el TRB crudo + `dequeue_idx` + ERDP al expirar) y
  `:628-645` (un reintento del puerto al final del escaneo).
- Linux: `xhci-ring.c:1649` → `:428` `xhci_abort_cmd_ring`; `xhci.h` `cmd_list`.
- Hecho en host: `cargo test -p xhci-nostd` con una completion de `ptr=0`: el
  slot se asigna y el puerto se reintenta una vez.
- Hecho en placa: el puerto 6 enumera (`slot N enabled`) o el log dice por qué
  no, con el TRB crudo.

### 5. Acotar el volcado del log (sondeo de WiFi)

- Archivos: `kernel/src/drivers/fatlog.rs:435` (`flush_inner_sync`) y
  `:89` (`flush_or_warn`): presupuesto por pasada (p. ej. un bloque) o salto
  mientras haya un HCMD de WiFi en vuelo.
- Linux: no aplica (aquí no hay `fatlog`); el criterio es el watchdog existente
  `kernel/src/lxdde/wifi.rs:151-160`.
- Hecho en host: `cargo xtask check`.
- Hecho en placa: ningún `sondeo parado … causa=volcado del log` mayor de 500 ms
  en el siguiente SOSOLOG.

Reflash (usuario; no lo ejecuta el agente):

```bash
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

## 5. Qué no se ha hecho

- No se ha tocado ningún driver: ni `iwl_mvm_nvm.c`, ni `xhci-nostd`, ni
  `fatlog`. El plan de arriba está sin implementar.
- No hay ciclo de placa de la corrección: no se ha reflasheado nada (ni el
  kernel, ni la ESP) y no se ha usado `record-boot` ni `--boot-ok`.
- No se ha determinado la causa exacta del rechazo de la NVM (H1 la deja
  instrumentada, no resuelta) ni por qué el controlador xHCI emite la completion
  del `Enable Slot` con `ptr=0`.
- No se ha probado nada de RF: sin `init_complete` no hay scan ni asociación
  reales, y el mensaje `wifi-wpa: fallo AUTH+ASSOC` no dice nada de la red.
- El ethernet `rtl8169` no se ha investigado como fallo: enlace DOWN por cable
  ausente (`bmsr 0x7989`).
