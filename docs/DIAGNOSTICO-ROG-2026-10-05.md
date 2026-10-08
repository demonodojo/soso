# ROG RTX 3080 Laptop: diagnóstico (5 oct 2026, segundo arranque)

## Evidencia y alcance

| Campo | Valor |
|---|---|
| ESP | `/dev/sda1` (`KERNEL`, vfat), desmontada tras la copia |
| Kernel arrancado | **soso 0.3.7 (f52ef0d6a-dirty)** |
| Manifiesto `SOSOHASH.TXT` | Sigue en el empaquetado de la mañana: `version=0.3.7`, `build=cd5f6a36a-dirty`, `perfil=nouveau,iwlwifi` (07:49) |
| Flush / uptime | **#14**, **8862248 ms** (~2,5 h en kernel-shell) |
| PCI | `10de:249c` GA104, `8086:2723` AX200, `10ec:8168` rtl8169, `1002:1638` framebuffer GOP |
| Rootfs | p2 `/dev/sda2`, superbloque generación **24** (el de la mañana montó generación **1**) |
| Copias | `target/usb-diagnostic-2026-10-05/SOSOLOG.TXT` (este arranque), `SOSOLOG.body.txt` (mañana, `cd5f6a36a`) |
| Árbol Linux | `lxdde/linux/` 6.6.32 (`r8169_main.c`). `drivers/scsi/sd.c` no está en ese árbol |
| Hostcheck | `l6-iwl-fw-hostcheck.sh` y `l6-g3-gsp-hostcheck.sh` salieron 0. No cubren transporte USB, el árbol sosofs ni esta placa |

`SOSOWIFI.TXT` vacío. `BOOTMARK.TXT` escrito (UEFI 2.70, American Megatrends). El arranque de la mañana, mismo día y mismo stick, sí llegó a sosh con `GSP_INIT_DONE`, `pool VRAM=sí` y `UCODE_ALIVE_NTFY` (`SOSOLOG.body.txt`). Este arranque no.

## Tabla de etapas

| Etapa | Cita SOSOLOG | Resultado |
|---|---|---|
| Shim | `soso-shim: UEFI alcanzado` | OK |
| Kernel hasta `boot: task` | `boot: task` | OK |
| sosofs | `fs: sosofs live (generación 24, 327680 bloques)` | Monta, árbol incoherente |
| Userspace | `init: sin /bin/init` y luego `soso>` | **Fallo** |
| GPU | `lx-nouveau ausente`; `GSP=off, pool VRAM=no`; `NV_PMC_BOOT_0=0xb74000a1` | **Fallo** (feature no enlazada) |
| WiFi | `lx-iwlwifi ausente`; `wifi: requiere feature lxdde` | **Fallo** (feature no enlazada) |
| Ethernet | `firmware PHY ausente`; `phystatus 0x04 … enlace DOWN` | Sin enlace y sin firmware |
| USB | mass storage `048d:1234`, `sync_cache=false`, sense `key=0x5 asc=0x24` | Enumera; barrera de caché no |

## Hallazgos

### 1. El commit 24 de sosofs deja huérfana la hoja de `/` (confirmado)

- **Síntoma:** el FS monta y a continuación no existen `/bin/init`, `/etc`, `/var/log` ni `/lib/firmware/rtl_nic/rtl8168h-2.fw`. La shell es la de emergencia. Por la mañana, con generación 1, `task: /bin/init lanzado (pid 1)`.
- **Disco (p2, lectura en el host):** el superbloque vigente es generación 24, `tree_root=312016`. Ese nodo (nivel 2, 68 hijos, CRC32C correcto, generación 24) manda el inodo 1 al hijo `275988`, un nodo de generación **464** cuya primera clave es el inodo 24972 y que el bitmap de la gen 24 marca libre. La hoja que sí tiene el directorio raíz es el bloque **275987** (generación 18, CRC correcto, 12 claves): dirents `bin`, `lib`, `etc`, `var`, `tmp`. En todo el volumen no hay ningún puntero de hijo a 275987 (la otra coincidencia de esos 8 bytes es el `expected_block` de la propia hoja).
- **soso:** `crates/sosofs/src/write.rs:211-223` reescribe el nodo interno tras el CoW (`v[idx] = (cow.min_key, cow.block)` y `free_block` del nodo viejo) y `commit` (`316-351`) publica el superbloque sin volver a leer el hijo ni comprobar que el inodo 1 sigue resolviendo. `kernel/src/main.rs:268-281` solo mira `resolve("/bin/init")`.
- **Linux:** no aplica; sosofs no tiene homólogo en `lxdde/linux`.
- **Causal:** **confirmado** para el síntoma. La raíz publicada no es un sector a medias (el CRC cuadra): el commit escribió un separador que no lleva a la hoja viva.

### 2. `SYNCHRONIZE CACHE(10)` con sense 5/24 se toma como barrera hecha (confirmado el sense; la pérdida de writes es hipótesis)

- **Log:** `SYNCHRONIZE CACHE(10) falló` → `sense slot=1 key=0x5 asc=0x24 ascq=0x00` → `sync_cache=false`.
- **soso:** el CDB es solo el opcode `0x35` (`crates/xhci-nostd/src/mass_storage.rs:29-32`). `kernel/src/drivers/usb_storage.rs:353-356` devuelve `Ok(())` si el probe dejó `sync_cache` en falso, con un comentario que atribuye a Linux `sd.c:2974-2979` el asc `0x24` como write-through.
- **Linux:** `lxdde/linux` es 6.6.32 y **no incluye** `drivers/scsi/sd.c`. Ese comentario no se puede contrastar en el árbol local. El asc `0x24` es *invalid field in CDB*, no *invalid command* (`0x20`).
- **Causal:** el sense y el `flush` que no falla están **confirmados**. Que eso haya fabricado la raíz del hallazgo 1 es **hipótesis**: encaja con un read-after-write que devuelve el bloque viejo y con un commit posterior que publica esa vista, pero el nodo de generación 24 tiene CRC bueno y punteros coherentes con lo que el escritor decidió guardar.

### 3. Este kernel no lleva nouveau ni iwlwifi (confirmado)

- **Log:** `drv: 01:00.0 10de:249c lx-nouveau ausente`, `drv: 03:00.0 8086:2723 lx-iwlwifi ausente`, `gpu: NVIDIA detectada (chipset None, GSP=off, pool VRAM=no)`, `wifi: requiere feature lxdde`. El mismo PCI por la mañana salió `compilado`, con `GSP_INIT_DONE` y `UCODE_ALIVE_NTFY`.
- **soso:** `kernel/src/drivers/registry.rs:154-166` marca `lxdde` solo con `cfg!(feature = "lxdde")`. Sin esa feature, `kernel/src/main.rs:201-210` no llama a `lxdde::init` y `nvidia_probe::init` corre después de `gpu::init` (`223-227`), que por eso imprime `chipset None` aun cuando el registro `NV_PMC_BOOT_0` sale bien en la línea siguiente. `xtask/src/package_live.rs:15-23` usa `SOSO_DRIVERS` si está definido; `preset_all` (`xtask/src/drivers.rs:59-65`) no trae puertos nouveau ni iwlwifi.
- **Linux:** el port no llegó a ejecutarse. No hay discrepancia de línea con `iwlwifi` ni con nvkm en este binario: la etapa no está enlazada.
- **Causal:** **confirmado** que el ELF no tiene la feature. **Hipótesis** que se haya empaquetado con `SOSO_DRIVERS=all` (o un `cargo xtask build` sin perfil live). El manifiesto de la ESP todavía describe el kernel de la mañana, que sí las tenía.

### 4. Ethernet DOWN (consecuencia, no la causa del kshell)

- **Log:** `rtl8169: firmware PHY ausente (/lib/firmware/rtl_nic/rtl8168h-2.fw)` y `phystatus 0x04 bmsr 0x7989 → enlace DOWN`. Por la mañana, con el mismo chip, el firmware cargó (`211 opcodes`) y el enlace también estaba DOWN (`phystatus 0x84`).
- **soso:** `kernel/src/drivers/rtl8169.rs:753-761` resuelve esa ruta por VFS; falla porque el hallazgo 1 no deja ver `/lib`.
- **Linux:** `lxdde/linux/drivers/net/ethernet/realtek/r8169_main.c:4637-4653` (`r8169_phylink_handler`) solo despierta la cola si `netif_carrier_ok`. Sin cable no hay tráfico IP.
- **Causal:** el DOWN de esta sesión mezcla firmware no leído (por el FS) y ausencia de carrier. No explica `sin /bin/init`.

## Orden de corrección

1. **No publicar un sosofs cuyo directorio raíz no resuelve.** En `crates/sosofs/src/write.rs`, `commit` (tras escribir nodos y antes del superbloque) debe invalidar la caché, releer cada hijo de la raíz y abortar si la clave mínima en disco no es el separador, si el bloque no está marcado en el bitmap, o si el inodo 1 no resuelve. `insert_rec` (`211-223`) es el sitio donde el separador se sustituye sin esa comprobación. Test de host: varios `append_file` sobre un inodo alto (el patrón de `logfs`) y `resolve` de un fichero creado en `/` sigue encontrándolo. En placa este rootfs ya está huérfano: después del arreglo hace falta reescribir la partición 2 (flash de rootfs, no `--only kernel`). Hecho en placa: `task: /bin/init lanzado` y generación nueva, no la 24.

2. **La barrera USB no puede devolver éxito cuando el sense es 5/24.** En `crates/xhci-nostd/src/mass_storage.rs` (`sync_cache10_cdb`) y `kernel/src/drivers/usb_storage.rs:346-363`: si `SYNCHRONIZE CACHE(10)` responde asc `0x24`, probar `SYNCHRONIZE CACHE(16)` (opcode `0x91`, LBA 0, longitud 0). Si tampoco, `flush` debe fallar y `Sosofs::commit` no debe avanzar el superbloque. `drivers/scsi/sd.c` no está en el árbol 6.6.32; no dar por bueno el comentario de write-through. Hecho en host: un dispositivo que rechaza el CDB de 10 bytes no deja `sync_cache=false` con `flush` en `Ok`. Hecho en placa: o `sync_cache=true`, o un commit que no sobrevive como generación 24 con la hoja de `/` descolgada.

3. **El kernel de live no puede salir sin nouveau ni iwlwifi.** `live_driver_profile` no debe heredar `SOSO_DRIVERS=all` / `preset_all` en silencio: si el perfil no incluye los puertos `nouveau` e `iwlwifi`, el empaquetado live tiene que negarse o usar `preset_live_usb`. Hecho en host: el hwscan del kernel que escribe `--only kernel` marca `lx-nouveau compilado` y `lx-iwlwifi compilado`. Hecho en placa (con el rootfs del punto 1 ya legible): `GSP_INIT_DONE` y `UCODE_ALIVE_NTFY`, como el arranque `cd5f6a36a` de esta mañana.

4. **Registro.** Matriz `ga107-igpu` y `ax200-wifi` e este informe, en esta ejecución. La racha de arranques seguidos queda en 0: este no llegó a sosh y el parser había contado `boot: task` como userspace.

## Qué no se ha hecho

No se ha tocado código de sosofs, USB ni drivers. No se ha reflasheado el USB. No se ha incrementado `arranques_consecutivos_ok`.
