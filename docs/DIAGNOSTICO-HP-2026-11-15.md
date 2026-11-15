# HP 8265: PHY_CFG con id equivocado (0x0b) y ESP sin hueco para `SOSOKRN.MET`

## 1. Evidencia y alcance

Lectura de la ESP del USB live con `udisksctl` (`/dev/sda1`, vfat, etiqueta
`KERNEL`, extraíble, 190 MiB, clúster de 512 B, 61 MiB libres). ESP desmontada
tras copiar. Copias en `target/usb-diagnostic-2026-11-15/` (`SOSOLOG.txt`,
`SOSODRV.txt`, `ESP-listado.txt`, huecos OTA). `SOSOWIFI.TXT` tiene `ssid`/`psk`
(el PSK no se reproduce aquí).

| Campo | Valor |
|---|---|
| Kernel en el USB | **soso 0.3.28 (`5902f927a`)** — sin `-dirty` |
| ESP flasheada | `SOSOHASH`: 0.3.28, build `886e8bb0d-dirty`, 2026-11-05 |
| `kernel-x86_64` | 2026-11-08 10:41 → último flash fue `--only kernel` |
| Flush | **#32 @ 259211 ms** (18 volcados en la cola, `fatlog`) |
| Firmware UEFI | HP rev 0x11700 (2.50); shim leyó `bootsoso.efi` 156672 B |
| Hardware | iGPU `8086:5917`, WiFi **`8086:24fd` (8265)**, `10ec:8168`, nvme `1c5c:1527`, xHCI `8086:9d2f` |
| Userspace | **`sosh —`** OK (pid 1 `/bin/init`), `soso-wm` lanzado |
| Red | sin IP en ningún backend: WiFi muere en MVM y rtl8169 está DOWN |

Árboles Linux usados (solo lectura):

| Árbol | Tag/commit | Uso aquí |
|---|---|---|
| `lxdde/linux/` | 6.6.32 | `mvm/fw.c` (PHY_CFG), `fw/api/commands.h`, `fw/api/config.h`, `fs/fat/{inode,file}.c` |

Hostchecks (`hostcheck-iwl.txt`, `hostcheck-gsp.txt` en la copia): los dos en
verde. El de iwlwifi ya parsea el TLV del 8265 (`OK: 8265 SEC_RT lmac/umac`),
pero **no envía comandos**: no ejerce la rama 8000 ni el id de PHY_CFG, así que
no habría detectado esto. El de GSP no aplica (esta placa no tiene NVIDIA).

No se valida: silicio real del 8265 más allá del ALIVE, transporte ICT/TFD
completo, ni el contenido de los slots que la ESP no puede crear.

---

## 2. Tabla de etapas (flush #32)

| Etapa | Evidencia en `SOSOLOG` | Resultado |
|---|---|---|
| Boot hasta `boot: task` | `boot: memtest … task`, `smp: 8/8 CPUs en línea` | OK |
| `sosh` | `sosh — escribe 'help' para la ayuda` | OK |
| Probe 8265 | `lxdde-pci: iwlwifi probe 8086:24fd ok`, `familia=8000` | OK |
| Transporte FH 8000 | `RELEASE_CPU_RESET`, kick CPU1/CPU2, `HCMD 8000` | OK |
| ALIVE INIT | `iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY)` | OK |
| ALIVE RT | `fw_alive 8000 SCD=0x00817080 qid=0`, ALIVE otra vez | OK |
| TX_ANT | `TX_ANT wide ver_hdr=0 len=12` → sin timeout (no hay `TX_ANT 8000 falló`) | OK |
| PHY_CFG | `iwl_trans: timeout cmd grp=1 id=0x0b seq=0x0001 slot=1; MVM parado` | **FAIL** |
| INIT_COMPLETE | `iwl_mvm: init MVM incompleto` | **FAIL** (no alcanzado) |
| Recuperación | 3 ciclos idénticos, `recuperación falló (… 3 de 3)` | **FAIL** |
| scan | `sosh: wifi scan: error de E/S` | **FAIL** |
| assoc WPA2 | `wifi-wpa: fallo AUTH+ASSOC 802.11` · `no pude conectar a 'Rutilo'` | **FAIL** |
| DHCP | no hay lease; `rtl8169: … enlace DOWN 10M half` | **FAIL** (sin medio) |
| Huecos OTA | `updslot: SOSOKRN.MET no está usable; lo repongo` → `no pude reponer SOSOKRN.MET (RaizLlena)` | **FAIL** |

Los tres ciclos de recuperación son idénticos: ALIVE → TX_ANT → timeout de
`0x0b` → cola envenenada (`cmd_needs_recover`) → recarga de firmware. No es
un fallo de firmware ni de alimentación: el FW arranca y contesta cada vez.

---

## 3. Hallazgos

### WIFI-1. `PHY_CONFIGURATION_CMD` vale `0x0b` en soso y `0x6a` en Linux — **confirmado, bloqueante**

**Síntoma:** `TX_ANT` pasa y el siguiente comando no contesta nunca:
`iwl_trans: timeout cmd grp=1 id=0x0b seq=0x0001 slot=1; MVM parado`
(precedido de `iwl_rx: sin emparejar (esperando grp=1 id=0x0b seq=0x0001)`).

**soso:** [`iwl_internal.h:433`](../lxdde/ports/iwlwifi/iwl_internal.h)
`#define PHY_CONFIGURATION_CMD 0x0b`, enviado en
[`iwl_mvm_init.c:63`](../lxdde/ports/iwlwifi/iwl_mvm_init.c) con
`LEGACY_GROUP` → cabecera wide con `group_id=LONG_GROUP` (1), que es justo el
`grp=1 id=0x0b` del timeout.

**Linux 6.6:** [`fw/api/commands.h:308`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/commands.h)
`PHY_CONFIGURATION_CMD = 0x6a`. `0x0b` no existe en `enum iwl_legacy_cmds`
(el vecino `0xb5` es `SCAN_ITERATION_COMPLETE_UMAC`), así que el FW no tiene
nada que responder y el driver espera 2000 ms en balde. La ruta unificada
(AX200/AX211) **omite** PHY_CFG —[`iwl_mvm_init.c:136-164`](../lxdde/ports/iwlwifi/iwl_mvm_init.c)—,
por eso el id erróneo nunca se había ejercitado: solo lo usa la rama 8000
añadida para este 8265.

**Compatible con el síntoma:** el orden TX_ANT → PHY_CFG es el correcto
(Linux `iwl_run_init_mvm_ucode`, [`mvm/fw.c:688`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c)
y `iwl_mvm_load_rt_fw`, `fw.c:1544`), y TX_ANT sí obtiene respuesta. El único
elemento fuera de sitio es el id. Confirmado.

### WIFI-2. Payload de PHY_CFG con layout y calibración que no son los de Linux — **confirmado, no causal aún**

**soso:** [`iwl_internal.h:1687-1696`](../lxdde/ports/iwlwifi/iwl_internal.h)
declara `struct iwl_calib_ctrl { ucode_control, flow_trigger, flow_block }`
(12 B) y `iwl_phy_cfg_cmd_v1` = 16 B, con `calib_control` a cero
(`memset` en `iwl_mvm_init.c:61`) y `phy_cfg = iwl->phy_sku` del ALIVE.

**Linux 6.6:** [`fw/api/config.h:34-37`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/fw/api/config.h)
`struct iwl_calib_ctrl { __le32 flow_trigger; __le32 event_trigger; }` (8 B) →
`iwl_phy_cfg_cmd_v1` = 12 B, o v3 (28 B) si `cmd_ver >= 3`. Y rellena
`phy_cfg` con `iwl_mvm_get_phy_config()` (cadenas/radio del NVM) **más**
`trans_cfg->extra_phy_cfg_flags`, y `calib_control.*` con
`fw->default_calib[cur_fw_img]` (`fw.c:556-574`), no con el `phy_sku` del ALIVE.

**Compatible:** hoy el comando ni se envía con el id correcto, así que este
desajuste no puede ser la causa de *este* log; sí será el siguiente muro
(FW rechazando el comando o nunca emitiendo `INIT_COMPLETE_NOTIF`) si se
arregla solo el id. Descartado como causa actual; anotado como dependencia.

### WIFI-3. `wifi::poll` parado 1,2–6,0 s por el volcado del log — **confirmado, no causal**

`wifi: sondeo parado 15 veces en 30000 ms (max 5142 ms, causa=volcado del log ~1198 ms)`.
El `fatlog` bloquea 1,5–5,9 s (`flush #32 … dur=1537ms max=5876ms`). Molesta al
usuario y falsea tiempos, pero el transporte ya estaba muerto antes; no explica
el timeout de `0x0b`.

### ESP-1. La raíz de la ESP no tiene hueco libre y `SOSOKRN.MET` no se puede reponer — **confirmado**

**Síntoma:** `updslot: SOSOKRN.MET no está usable; lo repongo` y
`updslot: no pude reponer SOSOKRN.MET (RaizLlena)`
→ `recuperación OTA limitada`. El hueco **no existe** en el listado de la ESP
(están `SOSOUPD.TXT`, `SOSOKRN.BIN`, `SOSOTXN.BIN`, `SOSOMODE.TXT`) y el
kernel tampoco puede crearlo.

**soso:** [`espfat-core/src/lib.rs:641-671`](../crates/espfat-core/src/lib.rs)
`hueco_directorio()` recorre la cadena de la raíz buscando una entrada
`0x00`/`0xE5`; si la cadena termina, devuelve `RaizLlena` **sin ampliarla**
(ruta de creación en `lib.rs:309-313`). `kernel/src/drivers/updslot.rs:53-75`
solo lo reporta.

**Evidencia de la ESP:** clúster de 512 B → **16 entradas por clúster**, y la
raíz ya necesita un segundo clúster. `ls -fa` lista 19 entradas, entre ellas
cinco con nombres imposibles y tamaños de 680 MB / 1,1 GB / 2,7 GB en una
partición de 190 MiB (`ESP-listado.txt`). Linux las muestra así porque son
entradas huérfanas/de relleno mal formadas: `ls` oculta las entradas LFN
(attr `0x0F`) cuyo checksum no casa, mientras que `entrada_libre()` de soso
las cuenta como ocupadas. Resultado: para Linux la raíz tiene sitio, para
soso está llena.

**Linux 6.6:** `fat_add_cluster()`
([`fs/fat/inode.c:104`](../lxdde/linux/fs/fat/inode.c)) amplía el directorio
cuando se llena: `fat_alloc_clusters(1)` + `fat_chain_add`, y el clúster nuevo
se entrega en ceros (`fs/fat/file.c:292`). soso tiene ese código **solo** en el
host, [`xtask/src/fat32_write.rs:971-990`](../xtask/src/fat32_write.rs)
(`grow_fat32_root`), no en `espfat-core`, que es lo que corre en la máquina.

**Origen de la basura (confirmado en el código):**
`delete_named_with_lfn()` solo marcaba `0xE5` las entradas LFN cuyo `checksum`
coincidía con el nombre 8.3 que se estaba borrando, y `install_entry_in_dir()`
deja las nuevas donde haya hueco. Cada reescritura de `kernel-x86_64`
(`ensure_kernel_lfn`) podía dejar huérfana la LFN anterior, y nada las
recuperaba nunca: el stick lleva 11 sesiones de diagnóstico y varios
`--only kernel`. Descartado al comparar: que `encode_lfn` no añada el terminador `0x0000`
cuando el nombre llena 13 caracteres **no** es un defecto — Linux tampoco lo
añade (`xlate_to_uni`, [`fs/fat/namei_vfat.c:560-573`](../lxdde/linux/fs/fat/namei_vfat.c):
solo si `outlen % 13`). La LFN estaba bien formada; lo que sobraba eran
entradas de nombres anteriores.

**Compatible:** sí. No es fragmentación ni falta de espacio (61 MiB libres).

### ETH-1. `rtl8169 … enlace DOWN 10M half` — **descartado**

`phystatus 0x84 bmsr 0x7989`: ni `phy & PHY_LINK_OK` ni `BMSR bit 2`
([`rtl8169.rs:1174-1179`](../kernel/src/drivers/rtl8169.rs)) están puestos, y
el MAC arrancó (`rtl_hw_start_8168h_1 (VER_46) ok`, firmware PHY 211 opcodes).
Es cable ausente, como en los diagnósticos del ROG y del HP de septiembre: no
es la causa de quedarse sin red (el DHCP se intenta sobre una NIC sin enlace).

---

## 4. Orden de corrección

Implementado en el árbol el 2026-11-15 con pruebas de host (los «hecho placa»
siguen pendientes de un SOSOLOG nuevo):

| # | Estado |
|---|---|
| 1 | **Hecho host.** `tools/iwl-hostcheck/mvm_init_test.c` comprueba el id.
| 2 | **Hecho host.** Prueba del payload v1 (12 B) y v3 (28 B).
| 3 | **Hecho host.** `crates/espfat-core/tests/raiz_llena.rs` (3 casos).
| 4 | **Hecho host.** Prueba de la LFN huérfana, verificada contra la lógica vieja.

1. **PHY_CONFIGURATION_CMD = 0x6a** en
   [`lxdde/ports/iwlwifi/iwl_internal.h:433`](../lxdde/ports/iwlwifi/iwl_internal.h).
   Discrepancia: Linux `fw/api/commands.h:308`.
   Hecho host: `cargo xtask lx-build iwlwifi` y una prueba que compare el id
   contra 0x6a (y que falle con 0x0b).
   Hecho placa: SOSOLOG con `timeout cmd grp=1 id=0x6a …` **fuera**, es decir
   `INIT_COMPLETE_NOTIF` y `init 8000 listo`.

2. **Payload de PHY_CFG como Linux** (`iwl_calib_ctrl` de 8 B
   `flow_trigger`/`event_trigger`; `phy_cfg` de `iwl_mvm_get_phy_config` +
   `extra_phy_cfg_flags`; `calib_control` del TLV `default_calib` de la imagen
   en curso, y v3 si `cmd_ver >= 3`) en `iwl_internal.h`,
   `iwl_mvm_init.c`, `iwl_fw.c`.
   Discrepancia: Linux `fw/api/config.h:34`, `mvm/fw.c:556-574`.
   Hecho host: `mvm_init_test.c` compara los 12 B de v1 (y los 28 de v3) con
   `phy_cfg` = `phy_sku` con las cadenas de las antenas válidas y la
   calibración del TLV.
   Hecho placa: `INIT_COMPLETE_NOTIF` sin tocar el id del comando.

3. **Ampliar la raíz FAT32 desde el kernel** en
   [`crates/espfat-core/src/lib.rs:641-671`](../crates/espfat-core/src/lib.rs):
   si la cadena acaba sin hueco, `buscar_run(1)` + marcar cadena + rellenar el
   clúster a cero + enlazar, como `fat_add_cluster` (`fs/fat/inode.c:104`).
   Con eso `SOSOKRN.MET` se repone en el siguiente arranque. Además el hueco
   de directorio se reserva **antes** de elegir el run de datos: si se elige
   primero el del fichero, el clúster que consume la ampliación acaba
   enlazado con la raíz y con el fichero a la vez.
   Hecho host: `cargo test -p espfat-core --features std` (clúster a ceros,
   sin espacio y no-compartir-clúster).
   Hecho placa: `updslot: SOSOKRN.MET repuesto, LBA …` y `locate` OK.

4. **No dejar LFN huérfanas** en
   [`xtask/src/fat32_write.rs`](../xtask/src/fat32_write.rs):
   `delete_named_with_lfn` borra por posición el tramo de LFN contiguo anterior
   a la entrada 8.3, mire o no el checksum (las de nombres anteriores ya no
   casan y se quedaban vivas para siempre). `encode_lfn` no se toca: Linux
   tampoco escribe terminador cuando el nombre llena 13 caracteres.
   Hecho host: prueba con `kernel-x86_64` que estropea el checksum de la LFN y
   exige que tras `ensure_kernel_lfn` quede **una** entrada larga, no dos;
   comprobado que falla con la lógica anterior.
   Hecho placa (stick nuevo o reparado): `ls -fa` sin entradas de nombre
   imposible y `RaizLlena` fuera del SOSOLOG.

5. **Reparar este stick** (usuario, no el agente): `fsck.vfat -a /dev/sda1`
   desde Linux y, si no basta, `cargo xtask flash-usb-live /dev/sda --yes
   --only kernel`. El agente no flashea ni escribe la ESP.

---

## 5. Qué no se ha hecho

La corrección 1–4 está en el árbol y verificada en host, pero **no se ha
probado en silicio**: falta reflashear (`cargo xtask flash-usb-live /dev/sda
--yes --only kernel`, lo lanza el usuario) y traer un SOSOLOG nuevo. El stick
sigue con la ESP tal cual: el agente no la montó en escritura ni la flasheó.
Sin `--boot-ok` ni incremento de `arranques_consecutivos_ok` (sosh no es
aceptación): la matriz solo cambió en `iwl8265-wifi`. `target/` no va al git.
