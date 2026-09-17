---
name: soso-live
description: >-
  soso live USB, native install and OTA updates — GPT image, ESP 8.3 slots,
  boot-shim, USB/xHCI mass storage, soso-install and soso-update. Use when
  modifying package-usb-live, flash-usb-live, boot-shim, updslot, bootreq,
  espfat, usb_storage, xhci-nostd, gptdisk, soso-update-core, or ESP files
  SOSOLOG/SOSODRV/SOSOBOOT/SOSOUPD/SOSOKRN/SOSOKRN.MET/SOSOWIFI.
---

# soso — Live USB, instalación y actualizaciones

## Plan aplicable y seguimiento

Al avanzar un plan de instalación, OTA, recuperación o automejora, leer
[Identificación y seguimiento de planes](../soso-architecture/references/planes.md).
Conservar el ID de entrega del plan activo y registrar imagen/build, destino
y alcance de la comprobación. Las fichas T31/T43 de automejora exigen evidencia
de recuperación de kernel y rootfs: no cerrarlas solo con rollback del kernel.
Mantener estado, artefactos y siguiente paso; un paquete preparado no acredita
haberlo instalado ni arrancado en placa.

El camino de placa: **un GPT** (`soso-live.img`), no tres imágenes sueltas.
Docs de usuario: `MANUAL-USUARIO.md`. Bring-up on-box: `docs/L5c-on-box.md`.
Guía operativa: [`docs/GUIA-OPERATIVA.md`](../../docs/GUIA-OPERATIVA.md).
Estado/límites OTA: [`docs/ESTADO.md`](../../docs/ESTADO.md).

Plan U0–U8: [actualizaciones de instalaciones y logs en sosofs](../../../docs/PLAN-ACTUALIZACIONES.md).
Incluye recuperación conjunta kernel/rootfs, transición legacy y eliminación
de `SOSOLOG.TXT` en la ESP instalada; no tratarlo como comportamiento implementado.

**U5e parcial (2026-09-17):** `aplicar` **se planta** si el registro dice que esa
versión ya hubo que deshacerla (pide `--forzar`), y al armar se recogen los
puntos que no referencia ni la versión activa ni la operación nueva — **nunca por
tiempo ni por espacio, y nada si no consta ninguna referencia**. Falta comprobar
el arranque restaurado y no entrar en bucle si también falla.

**U5d parcial (2026-09-17):** `soso-update revertir` va contra la transacción —
muestra A→B, **verifica el punto releyéndolo**, deja cancelar y registra
`Decision::Rescatar`; restaura el arranque siguiente (`txn: restaurada la
versión`). Falta la **entrada UEFI** independiente de init/red. Trampa cazada
aquí: un `Contenido` con hash vacío se escribe `0 ` y al releer se recorta el
espacio → campo impartible y registro **ilegible**; «no consta» va explícito
(`- -`). Vale para el punto y para el diario.

**U5c cerrada (2026-09-17): `soso-update` ARMA, no instala.** Baja a la etapa,
crea+verifica el punto, escribe el **diario**, prepara el kernel y publica el
**registro de arranque**; instala `drivers/txnaplica.rs` en el arranque
siguiente, antes de firmware e init. `/etc/soso-release` es un fichero
administrado más (si no, volver atrás deja binarios viejos anunciando versión
nueva). `init` confirma la pareja con `SYS_TXN_CONFIRM` (89) — **sin eso el
arranque siguiente ve `probando` y deshace la actualización**. El banco exige
`txn: actualización aplicada` en el serial **antes** de `boot: ethernet`.
Trampas encontradas: el diario con hash de kernel vacío queda **ilegible** y
aparece como «armada sin diario» (la imagen live no trae `kernel=` en
`/etc/soso-release`); y al rearmar la misma release hay que **continuar la
secuencia** del diario o `pick` elige el obsoleto. Falta: arrancar la pareja
antigua **bajo su kernel** (sigue en `SOSOKRN.MET`) y cortes E2E.

**Cliente creando puntos (2026-09-17):** `soso-update` implementa `Almacen` y
crea+verifica el punto **antes de tocar el sistema**; si no se puede, no
actualiza. `estado` muestra la vuelta atrás y si está verificada. Huecos ESP
nuevos: `UPD_WHICH_TXN` (SOSOTXN.BIN) y `UPD_WHICH_MODE` (SOSOMODE.TXT, de donde
sale el GUID que ata el punto a su instalación). Ojo: `sys::getdents` devuelve
**bytes**, no entradas — dividir por `DIRENT_SIZE`.

**U5b cerrada (2026-09-17):** `punto::crear_punto` comprueba espacio **antes**
de copiar, copia y **relee** todo; sin espacio, con una copia que falla o que no
se relee igual, devuelve error y **no se arma**. `Retencion` conserva A mientras
C no confirme (dos puntos a la vez a propósito) y **no suelta el viejo si el
nuevo no está verificado**. Falta implementación real de `Almacen`: quien cree
puntos de verdad será el cliente al armar.

**U5a cerrada (2026-09-17):** registro de arranque en **formato 2** — punto
retenido + decisión `rescatar` — que **sigue leyendo el formato 1** (sin punto:
«no consta», no «no hay»; `BootRecord::conoce_puntos()`) y rechaza uno más nuevo
entero. `txn/punto.rs`: el punto guarda A **mientras B sea la activa**, aunque B
esté confirmada; lleva GUID del destino, se **verifica releyéndolo**, anota lo
que añadió B para **quitarlo** al volver, y `reserva_efectiva` cuenta los datos
dos veces por el CoW. `recogible` sólo suelta lo no referenciado (en A→B→C
conviven dos). La fila `rescatar` de la tabla **manda sobre el diario**. Nadie
crea puntos todavía: eso es U5b; la entrada UEFI que los pide, U5d.

**U5c parcial (2026-09-17) — no confundir con U5 entera.** El plan creció el
2026-09-16 (§3.6: puntos de recuperación **retenidos**, tres vías de vuelta
atrás, entrada UEFI de rescate) y U5 se desglosó en U5a–U5e, todas pendientes.
Hecho y probado: `txn/aplicador.rs` (idempotente, verifica hashes antes de
escribir, 11 pruebas cortando en **cada** paso) y `kernel/src/drivers/txnaplica.rs`
(lee `SOSOTXN.BIN` + diario, `reconcile`, ejecuta; corre tras montar sosofs y
**antes** de firmware e init; con pareja incoherente no lanza userspace).
**Está enlazado pero inerte:** `soso-update` todavía instala él mismo en vez de
armar, así que nadie publica un registro y `reconcile` siempre dice «normal».
Aviso de diseño: la reversión restaura **toda** la operación, no sólo lo marcado
como aplicado — entre escribir un fichero y anotarlo hay una ventana, y deshacer
sólo lo anotado dejaba una pareja mezclada. Feature `txn` en soso-update-core
para que el kernel no arrastre el formato de release.

**U4 cerrada (2026-09-17):** la descarga va a `/var/lib/soso-update/<id>/etapa/`
y **no toca el sistema activo** hasta tenerlo todo verificado; antes se escribía
`/bin`/`/lib` según llegaba cada tramo. Reanudable: el registro de la etapa está
atado al **hash del manifiesto** (no a la versión), lo verificado no se vuelve a
pedir y un fichero truncado no cuenta. RAM acotada por `TROZO_MAX` = 1 MiB
(`net::https_download_span_a` entrega por trozos); se siguen pidiendo **tramos**
para no hacer una petición por binario. `SYS_FSINFO` (88) da el espacio libre al
`preflight` de U0. Acreditado por `test-update`, cuyo arranque 2 ensucia el
fichero y reaplica para exigir reanudación **cruzando el reinicio**.

**U3 cerrada (2026-09-16):** el manifiesto lleva contrato de compatibilidad
(`arch`/`perfil`/`drivers`/`abi`/`fs`/`min_shim`/`min_recuperador`) que emite
`cargo xtask release`, y `soso-update` lo exige antes de descargar — **ojo al
sentido**: el manifiesto declara lo que la release **trae** y el equipo lo que
**necesita**; invertirlo rechaza cualquier release completa. Canales en
`crates/soso-update-core/src/canal.rs` con precedencia fija
(`--local` > `--channel` > `url=` > `channel=` > stable); el origen se resuelve
**una vez por comando**. Exclusiones del pack clasificadas por motivo
(`por_que_se_excluye`), ya con `var/log/` y `var/lib/soso-update/`; `release`
aborta si una ruta prohibida llega al pack. Las releases **no se firman**
todavía: ver la política en `docs/U0-CONTRATO-ACTUALIZACION.md` §8.

**U2 cerrada (2026-09-16):** la lógica FAT de la ESP vive en
`crates/espfat-core` (no_std, sobre un trait de sectores, 10 pruebas host) y el
kernel la usa a través de `drivers/espfat.rs`. `soso-install` **finaliza** el
destino: escribe `SOSOMODE.TXT` con modo `installed` y el GUID nuevo, **borra**
`SOSOLOG.TXT` y `SOSOBOOT.TXT` —borrado real: entrada de directorio + cadena FAT
en todas las copias; a ceros no vale—, rechaza clonar un origen con OTA a medias
y **pausa el escritor de logs** mientras copia (`sys::log_quiesce`), porque desde
U1 el kernel escribe el rootfs cada 2 s. `drivers/modo.rs` lee la identidad antes
de montar sosofs y `fatlog` se apaga **sólo** con identidad explícita, propia e
`installed`. Acreditado por `cargo xtask test-install`. No probado E2E:
`install-disk` y el `install-soso.sh` empaquetado, que hacen lo mismo por su
cuenta. Migrar instalaciones ya existentes es U6.

**U1 cerrada (2026-09-16):** logs nativos en sosofs —`/var/log/{kernel,
aplicaciones,actualizaciones}.log`— con cabecera por arranque, rotación 1 MiB × 3
y suspensión ante errores de FS (`kernel/src/drivers/logfs.rs`, lógica en
`crates/soso-log-core`). `sosolog` / `SYS_FATLOG_FLUSH` y `dmesg save` persisten
en **los dos** destinos. **Ojo:** en el live se escriben los dos a la vez, porque
la identidad `live`/`installed` no existe hasta que U2 cree `SOSOMODE.TXT`; hasta
entonces `fatlog` no se apaga en ninguna parte. Acreditado por
`cargo xtask test-update` (el arranque 2 comprueba que los registros del anterior
siguen ahí).

**U0 cerrada (2026-09-16), sólo banco host:** el contrato de la transacción está
en [U0-CONTRATO-ACTUALIZACION.md](../../../docs/U0-CONTRATO-ACTUALIZACION.md) y
en `crates/soso-update-core/{record.rs,txn/,identity.rs,compat.rs}`. Es lógica
`no_std` sin E/S: **no hay aplicador en el arranque**. Los ficheros ESP nuevos
(`SOSOTXN.BIN` y `SOSOMODE.TXT`, 4 KiB cada uno, cuatro ranuras) todavía **no**
los pre-crea `package-usb-live`; los reservan U2/U5. El buzón `SOSOUPD.TXT` y
`SOSOKRN.MET` siguen siendo el camino real del OTA de kernel. Siguiente: U1.

## Particiones (orden real en el stick)

| # | Nombre | Contenido |
|---|--------|-----------|
| 1 | ESP (FAT) | `BOOTX64.EFI` (shim) + `bootsoso.efi` + huecos 8.3 |
| 2 | sosofs | rootfs |
| 4 | `SOSOINSTALL` (FAT) | `install-soso.sh` para Linux; **tras el rootfs, no al final** |
| 3 | sosomfs | modelos (se estira al flashear; grow del superbloque al montar) |

Linux monta p4, no la ESP. El log **no** está en p4. El agente monta p1 con
`udisksctl` (abajo); el usuario puede usar `cargo xtask sosolog`.

Al final del pendrive Linux no montaba p4 (LBA que leía ceros). Por eso
`SOSOINSTALL` va entre rootfs y modelos.

## ESP en el host (agente: sin el usuario)

Linux **no** monta sola la ESP (p1 EFI). El volumen que sí aparece es p4
`SOSOINSTALL`; ahí no hay `SOSOLOG.TXT` ni `SOSOWIFI.TXT`.

`cargo xtask sosolog` llama a `sudo mount`. En Cursor **no hay TTY** (`sudo: a
terminal is required to read the password`): el agente **no** lo lanza, igual
que `flash-usb-live`. El usuario sí puede.

El agente monta p1 con **udisks** (polkit en extraíble: sin sudo ni
contraseña). `--no-user-interaction` evita el diálogo polkit.

> **Sandbox de Cursor sin D-Bus:** dentro del sandbox `udisksctl` falla con
> `Error connecting to the udisks daemon` / `Failed to connect to bus` y
> `/dev/sda1` ni siquiera aparece (`No such file or directory`). Este entorno
> no arranca con systemd como PID 1, así que **no hay bus de sesión visible**.
> `sudo mount` tampoco vale: pide contraseña y no hay TTY.
> **Solución (lo que sí funciona):** ejecuta el comando **fuera del sandbox**
> (`required_permissions: ["all"]`) y **exporta el bus de sesión** del usuario
> antes de `udisksctl`:
>
> ```bash
> export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/1000/bus"
> udisksctl mount -b /dev/sda1 --no-user-interaction   # → /media/jmdiez/kernel
> ```
>
> (uid 1000 = usuario de sesión; ajústalo si difiere). Con eso el agente monta,
> copia y desmonta sin dejarle el comando al usuario. `sudo` sigue vetado.

```bash
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTTYPENAME,RM
# Candidato: RM=1, partición 1, vfat / EFI. Etiqueta típica `kernel`.
# No uses p4 (`SOSOINSTALL`).

# Fuera del sandbox (required_permissions ["all"]) + bus de sesión:
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/1000/bus"

# ¿Ya montada? (p. ej. /media/<user>/kernel)
findmnt -n -o TARGET /dev/sda1

# Si no:
udisksctl mount -b /dev/sda1 --no-user-interaction
# → Mounted /dev/sda1 at /media/<user>/kernel

ESP=$(findmnt -n -o TARGET /dev/sda1)
cat "$ESP/SOSOLOG.TXT"
cat "$ESP/SOSODRV.TXT"
# Escritura (SOSOWIFI.TXT, …): el VFAT queda con uid del usuario de sesión.

udisksctl unmount -b /dev/sda1 --no-user-interaction
```

Si aun con el bus exportado `udisksctl` falla (auth, no extraíble), deja el
comando al usuario. **No** uses `sudo mount` ni `sudo cargo xtask sosolog`.

Tras leer o editar, **desmonta**: un `dd` posterior necesita p1 libre.

## Ficheros ESP (8.3, contiguos, pre-creados)

El kernel **no crea** ficheros en FAT: `espfat::locate` busca entrada 8.3 con
tamaño fijo y escribe sectores. Si falta el hueco, esa vía queda desactivada
(hay que reflashear).

| Fichero | Tamaño | Quién escribe | Para qué |
|---------|--------|---------------|----------|
| `BOOTMARK.TXT` | — | boot-shim | UEFI nos ejecutó; si sigue vacío, el firmware no arrancó el USB |
| `SOSOLOG.TXT` | 256 KiB | kernel `fatlog` (~2 s) | log serie persistente |
| `SOSODRV.TXT` | 16 KiB | `hwscan` / `drvlog` | informe PCI |
| `SOSOBOOT.TXT` | 4 KiB | `soso-install` → shim | `INSTALL <guid-ESP>` → `Boot####` |
| `SOSOWIFI.TXT` | 4 KiB | usuario en host | `ssid=` / `psk=` |
| `SOSOUPD.TXT` | 4 KiB | `soso-update` / shim / init | buzón OTA kernel |
| `SOSOMODE.TXT` | 4 KiB | `soso-install` / `install-disk` | identidad `live`/`installed` (U2); 4 ranuras de 1 KiB |
| `SOSOTXN.BIN` | 4 KiB | (U5) | registro de transacción del contrato U0; **reservado, aún sin usar** |
| `SOSOKRN.BIN` | 64 MiB | `soso-update` / shim | hueco kernel (nuevo o backup) |
| `SOSOKRN.MET` | 512 B | `soso-update` / shim | meta durable OTA (fases staged/backup/applying/probando) |
| `SOSORES.TXT` | 4 KiB | kernel `fs-resize` | journal de redimensionado rootfs |
| `kernel-x86_64` | **64 MiB** | `package-usb-live` (hueco) / `--only kernel` in situ | kernel UEFI activo; holgura para crecer sin flash completo |

Pass 1 de `package-usb-live` reserva huecos; pass 2 **reutiliza** la misma
entrada FAT (no duplicar `SOSOUPD`/`SOSOKRN`).

## Boot-shim (`boot-shim/`)

`BOOTX64.EFI` deja marca en `BOOTMARK.TXT`, atiende buzones y chainloadea
`efi/boot/bootsoso.efi`.

- **Install:** `bootentry.rs` lee `SOSOBOOT.TXT`, crea `Boot####` «soso».
  El kernel no puede tocar NVRAM (`ExitBootServices`). Si falla, **no aborta**
  el arranque. Deja `DONE Boot#### soso`.
- **Update:** `actualiza.rs` lee `SOSOUPD.TXT` y `SOSOKRN.MET`:
  - `KERNEL <tam> <sha256> <ver>` → verifica hueco, copia kernel viejo a
    `SOSOKRN.BIN`, escribe el nuevo, deja `PROBANDO` (meta `applying`/`probando`).
  - Corte en applying/backup → recuperación con backup verificado (tamaño/hash).
  - `PROBANDO` en el *siguiente* arranque = el anterior falló → revertir.
  - `OK` / `REVERTIR` / idle: ver `crates/soso-update-core/src/mailbox.rs`.
- Init confirma el kernel nuevo (`OK`) solo tras rootfs accesible, `spawn` de
  `/bin/sosh`, marca `/tmp/sosh-ready` con `pid=<pid>` de *esa* instancia
  (sosh registra fallo de mkdir/write/close y tiempos de prefault) y
  `kill(pid, 0)`. Si la marca tarda, init sigue sondeándola en el bucle de
  PID 1; no confirma por banner ni por un timeout más largo. Una marca vieja
  (`ok\n` u otro pid) no vale. Si sosh vive sin marca válida, no se confirma
  OTA y la shell no se mata.
  ELF perezoso: el PID de `spawn` no basta. Un page-fault de un binario
  lanzado después de la marca (no de sosh) no revierte el buzón.

Diagnóstico: `BOOTMARK` vacío → firmware; `BOOTMARK` escrito y `SOSOLOG`
vacío → kernel (checkpoints `boot:` en pantalla).

## Instalación nativa (`soso-install`)

Userspace: `user/coreutils/src/bin/soso-install.rs`.

1. `list` (o sin argumentos) — `SYS_DISK_LIST`; clasifica uso (live, soso,
   Linux, Windows, vacío) y particiones. Sin destino, pide al usuario cuál.
2. Destino **solo NVMe**, nunca el disco de arranque; `--force` si hay otro SO.
3. Clona el live (prefijo usado hasta el fin de los modelos empaquetados;
   omite cola vacía de p3); `gptdisk::relayout` (respaldo GPT al final, p3
   estirada, **GUID nuevos** — si no, el firmware no distingue USB y destino).
   Transferencias USB: READ(10) hasta 512 KiB con TRBs encadenados; progreso
   MiB/s en consola.
4. `SYS_BOOTREQ_WRITE` → `INSTALL <guid>` en `SOSOBOOT.TXT`.
5. Reiniciar **con el USB**: el shim registra `Boot####`. Quitar USB.

Tras el primer hwscan en live, el kernel escribe `/etc/soso-hw` (PCI+ncpu); el
clon lo lleva al NVMe. Arranques nativos omiten hwscan/`SOSODRV` si coincide;
`echo force > /etc/soso-hw` fuerza reescaneo.

Syscalls: `disk_list=37`, `disk_read=38`, `disk_write=39` (512 B/LBA;
`raw_disk` solo deja escribir NVMe y **nunca** el disco de arranque),
`bootreq_write=41` / `bootreq_read=42` (solo el fichero pre-creado).

Host alternativo: `cargo xtask install-disk /dev/nvmeXn1 --yes` (GRUB).

E2E: `cargo xtask test-install` (3 arranques OVMF; NVMe falso con swap/ESP
que el instalador debe rechazar). `SOSO_MODELS_SIZE=256M` para que sea rápido.

## OTA (`soso-update`)

Userspace + `crates/soso-update-core`. Release: `manifest.txt` + `rootfs.pack`
+ `kernel-x86_64` (`cargo xtask release [--publish]`).

- Rootfs: pack concatenado; `PackWriter::should_pack` salta rutas en `PACK_SKIP`.
  Sin rollback automático de binarios; progreso en `/etc/actualiza.estado`.
- Kernel: `SYS_UPD_WRITE`/`READ` (68/69) sobre huecos ESP + `SOSOKRN.MET`.
- Comandos: `estado` / `comprobar` / `aplicar` / `revertir`; `--local` apunta
  a un directorio (p. ej. `/var/actualiza-prueba`).
- Versión: `VERSION` → `/etc/soso-release` + banner kernel.

E2E: `cargo xtask test-update` (apply, corte simulado + recovery, manifiesto inválido).
Host: `cargo test -p soso-update-core --features std --tests`.

## USB / xHCI (`crates/xhci-nostd` + `drivers/usb_storage.rs`)

Live en placa: GPT por **USB BOT** (`live: GPT backend=Usb`) o NVMe.

La geometría GPT (p2/p3) se cachea en RAM tras el primer parse. Las lecturas
y escrituras de bloques **no** releen la tabla. Tras un resize/recovery que
reescriba GPT, `live_disk::refresh_geometry()` actualiza la caché; hasta
entonces los lectores siguen los límites anteriores (no una GPT a medias).

- DMA del event ring en **uncacheable** (`dma::alloc_zeroed_uc`). Write-back
  dejaba `EINT=1` y el software veía el anillo vacío (timeout + dump de puertos).
- **Longitud de Normal TRB = 17 bits:** `0x20000` (128 KiB) se desborda a 0 →
  Stall (`CSW inválido sig=0`). Tope `MAX_XFER = 64 KiB` en mass_storage.
  `raw_disk` usa 128 KiB y **debe** trocear antes del TRB.
- Bounce DMA **persistente** (`XhciController::bounce`): el asignador DMA del
  kernel no libera; uno por comando tiraba tanta RAM como datos movidos.
- **Babble en lecturas grandes de modelo (ABIERTO, placa ROG, qwen3.8-27b).**
  Síntoma: `bulk slot=1 dci=3 len=13 … (Babble Detected)` — babea el **CSW de
  13 B**, no la fase de datos — tras un READ(10) profundo en p3 (LBA válida, dentro
  de capacidad). La lectura del shard falla → `mmap-fault: lectura FS falló` →
  **page fault** de `/bin/soso-llm` (askd, pid 3) → y luego el flush del log a la
  ESP del mismo USB expira a 5 s en bucle (`transfer event timeout … dci=2` /
  `EP recovered`). **QEMU no lo reproduce** (nunca babea).
  - `mass_storage::bot_reset` (USB MS Bulk-Only §5.3.4 / Linux `usb_stor_Bulk_reset`):
    Bulk-Only Mass Storage Reset + `CLEAR_FEATURE(HALT)` en ambos endpoints bulk +
    `reset_and_requeue_ep`, con **reintento acotado** en `Bot::Error` de
    READ(10)/WRITE(10). Necesario (antes sólo EP0 se recuperaba) pero **NO
    suficiente**: en placa el `BOT reset` sale `recuperado` (control OK) y la
    **misma LBA vuelve a fallar**; a veces el propio EP0 (dci=1) expira, y
    `USBSTS=0x18` trae el bit **PCD** (Port Change Detect) → el dispositivo se
    atasca/cae por debajo del BOT (posible re-enum/brown-out en lecturas
    sostenidas grandes).
  - Diagnóstico añadido para el próximo arranque en placa: `residuo=` en el warn de
    bulk, `fase de datos corta n/want` en `bot_in`, `count=` y `log_ports()`
    (PORTSC/CCS/PLS) en el error de transporte. Decidirá: fase de datos corta →
    bug de residuo; `CCS=0` → desconexión (necesita re-enumeración); `CCS=1`+PLS en
    error → wedge de enlace. Hipótesis pendiente de datos: lecturas encadenadas
    (>64 KiB, TD multi-TRB) o brown-out → probar `MAX_READ_XFER = MAX_XFER`.
- `HOSTS` es `spin::Mutex` **no reentrante**. IRQ 1 / `poll_keyboard` no puede
  `lock()` (teclado muerto en placa; QEMU SSH no lo ve). Ver architecture.
- Se conservan todos los xHCI (un HCD por controlador, como Linux).

Tests: `cargo xtask test-usb` (4 escenarios en paralelo).

## Comandos host

```bash
cargo xtask package-usb-live              # qwen2.5-coder-3b Q4_K_M (sin medir stick)
cargo xtask flash-usb-live /dev/sdX --yes # mide, elige GGUF, dd, estira p3
SOSO_LIVE_OFFLINE=1 cargo xtask flash-usb-live /dev/sdX --yes
# incremental (pendrive ya flasheado; no toca p3 modelos):
cargo xtask flash-usb-live /dev/sdX --yes --skip-models
cargo xtask flash-usb-live /dev/sdX --yes --only kernel
SOSO_QEMU_LIVE=1 cargo xtask run
cargo xtask sosolog [--drv] [/dev/sdX]    # usuario (sudo/TTY); agente: udisksctl, sección ESP
cargo xtask test-install
cargo xtask test-update
cargo xtask test-usb
cargo xtask check
cargo xtask hw-matrix show
```

> **`--only kernel`:** actualiza `kernel-x86_64` (y `efi/boot/{bootsoso,bootx64}.efi`)
> **in situ**. El empaquetado live deja p1 **192 MiB** y un hueco de kernel
> **64 MiB** (como SOSOKRN). Si el hueco del stick es más pequeño, intenta
> agrandarlo; si no hay clusters, el ELF tiene que caber o hace falta un
> flash completo. No hace `dd` de `soso-uefi.img` p1 (~31 MiB) sobre la ESP
> live (run17). Ver `docs/DIAGNOSTICO-ROG-2026-09-10.md` run17.

No `sudo cargo`: root no tiene rustup. `cargo xtask flash-usb-live …` pide
sudo solo para grabar el disco (HOME/PATH salen de `SUDO_UID` si ya eres root).

**El agente no graba el USB.** El sudo del `dd` pide contraseña y en Cursor no
hay TTY (`sudo: a terminal is required to read the password`). Compila el
kernel, deja el comando exacto al usuario y no reintentes `sudo`.
`lx-build` y el cargo del kernel/userspace bajo sudo escriben `target/` como
el invocador (`SUDO_UID`), no como root. El `dd` al USB sigue siendo root.
Leer/editar la ESP: sección **ESP en el host** (`udisksctl`, sin sudo).

**Tras un arranque de placa**, con el pendrive de vuelta: actualizar la
matriz A8 (`docs/hw-matrix.json`) con `parse-logs` — skill **soso-dev**.
El agente puede editar ese JSON. No pongas etapas en `ok` sin evidencia
en SOSOLOG (`UCODE_ALIVE_NTFY`, GSP RPC). Un arranque a sosh no es
aceptación (hacen falta 3 seguidos + sesión).

Notas de sesión VFIO/kdump archivadas: [`docs/historico/notas-placa.md`](../../docs/historico/notas-placa.md)
(puntero desde [`board.txt`](../../board.txt)).

Perfil default live: `live-usb` (virtio + e1000e + rtl8169 + nvme + usb +
live-disk + nouveau + iwlwifi). Override: `SOSO_DRIVERS`.
