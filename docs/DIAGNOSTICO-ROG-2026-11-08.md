# Diagnóstico ROG — 2026-11-08

El live llega a `sosh` y el volcado se corta a los 94 s, en mitad del scan WiFi. Nadie pulsó una tecla (`kbd sc=0`). El kernel de este USB ya sondea el HID desde el planificador y el cuelgue sigue. La GPU completa GSP, el pool de VRAM y el CE. El semáforo 3D sigue en 0. No hay asociación ni DHCP.

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Pendrive | `/dev/sda`, ESP p1 `KERNEL`. Montada en solo lectura y desmontada |
| Copia | `target/usb-diagnostic-2026-11-08-0824/` (no va a git) |
| Log | flush #19, uptime 93 927 ms, 54 828 bytes útiles. `kbd sc=0 ultimo=0x00 enc=0 ent=0`. `dur=1311ms max=2710ms` |
| Kernel | `soso 0.3.28 (38e4298c8-dirty)`. El `-dirty` es `kbd::poll_devices` en `schedule_inner` |
| Manifiesto | `SOSOHASH` sigue en `886e8bb0d-dirty`, empaquetado 2026-11-05. El kernel se actualizó in situ |
| Rootfs | sosofs generación 52, 1 736 704 bloques; modelos `qwen27-iq3` |
| PCI | `10de:249c` GA104 (RTX 3080 Laptop, 16 384 MiB), `8086:2723` AX200, `10ec:8168` rtl8169 |
| Teclado | USB HID `0b05:1866` y `0b05:18c6` en el xHCI `06:00.3`. El pendrive está en `06:00.4` |
| Shim | `BOOTMARK` (8 nov 08:24): `soso-shim: UEFI alcanzado`, `bootsoso.efi` 156 672 bytes |
| Linux | `lxdde/linux` 6.6.32 (`drivers/net/wireless/intel/iwlwifi/iwl-io.c`, `drivers/usb/host/xhci-ring.c`) |

No se ha flasheado en esta sesión. El rollback a v0.3.22 está compilado en el worktree `soso-rog-v0.3.22` (rama `rollback/rog-v0.3.22`, commit `4790ea18b`): kernel 36 933 912 bytes, rootfs `target/soso-data.img` de 256 MiB. `main` no se ha movido.

## 2. Etapas

| Etapa | Cita del SOSOLOG | Resultado |
|-------|------------------|-----------|
| shim | `BOOTMARK`: `bootsoso.efi leído (156672 bytes)` | ok |
| userspace | `sosh — escribe 'help' para la ayuda` | llega, y el log se para |
| teclado | `kbd: usb hid activo (2)`; cabecera `kbd sc=0` | fallo: ninguna tecla |
| GPU GSP | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `GSP=rm_compute, pool VRAM=sí` | ok |
| CE | `CE readback verificado (G4e GO)` | ok |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)` | fallo, no bloquea sosh |
| WiFi | `UCODE_ALIVE_NTFY`; `alive=true`; `SCAN_REQ_UMAC v15`; el log acaba en `scan_count=17` | scan sin `scan fin` |
| Asociación | no hay selector ni `scan fin` | no se intentó |
| DHCP | no hay `net: dhcp` | pendiente |
| Ethernet | `rtl8169: … enlace DOWN 10M half` (`phystatus 0x84`) | driver arriba, sin cable |

## 3. Hallazgos

### H1 — el planificador ya sondea el HID y el teclado sigue en cero (confirmado en el log)

La cabecera es `kbd sc=0 ultimo=0x00 enc=0 ent=0` a los 93 927 ms. Los dos teclados ASUS quedan `keyboard ready` (`0b05:1866`, `0b05:18c6`). El banner lleva `-dirty`: este kernel ya llama a `kbd::poll_devices` en la BSP antes de coger `PROCS` (`kernel/src/task/mod.rs`). Ese sondeo no entregó ni un scancode.

El HID no tiene IRQ de xHCI. Linux entrega esos eventos en `xhci_irq` (`lxdde/linux/drivers/usb/host/xhci-ring.c:3093`, 6.6.32). soso solo los ve si alguien llama a `poll_keyboard`, y esa llamada hace `HOSTS.try_lock()` (`kernel/src/drivers/usb_storage.rs`). Mientras la BSP está dentro del scan, el planificador no corre y el parche no se ejecuta.

### H2 — el scan no imprime `scan fin` porque la BSP no vuelve al planificador (confirmado en el log; la causa del bucle es hipótesis)

`iwl_mvm_scan` imprime `scan fin` al salir del bucle de 400 pasos o de 8 s (`lxdde/ports/iwlwifi/iwl_mvm.c`). Este volcado tiene `SCAN_REQ_UMAC v15` y `scan_count` hasta 17, y no tiene `scan fin`. `fatlog::poll` solo corre en la BSP. El scan se hace en la syscall de `sosh`, en esa misma BSP, con `TRANS_LOCK` cogido (`kernel/src/lxdde/wifi.rs`). Hasta que el bucle vuelve, no hay otro flush: el #19 es el último porque el stick se sacó con la BSP aún dentro.

La espera de ese bucle es `iwl_mdelay_locked` (`lxdde/ports/iwlwifi/iwl_poll.h`), que solo llama a `lx_udelay`. Linux hace lo mismo en el sondeo de un bit: `udelay` y una lectura (`iwl-io.c:55-59`). No es una diferencia con Linux. Lo que Linux no hace es dejar el teclado USB sin su IRQ mientras esa espera gira.

El arranque de las 00:46, con `38e4298c8` sin `-dirty`, sí imprimió `scan fin count=19 end=1` y se quedó en el selector, también con `kbd sc=0` (`docs/DIAGNOSTICO-ROG-2026-11-07.md`, sección 8). El parche del planificador no explica por qué este volcado no llega a `scan fin`.

### H3 — errores de escritura en pantalla, ausentes del SOSOLOG (hipótesis)

En pantalla salen errores de escritura de sosofs o de `/var/log`. Este `SOSOLOG` no contiene `fatlog: aviso`, `usb write` ni un panic. El rootfs, `/var/log` y `SOSOLOG.TXT` se escriben en el mismo pendrive. Un write BOT deja `HOSTS` cogido toda la transferencia; el teclado del otro xHCI también pasa por ese candado (`usb_storage.rs`, `poll_keyboard_event`). Encaja con teclas perdidas durante las escrituras. No está confirmado como la línea que falla: el log no la cita.

### H4 — el enlace ethernet está caído porque el PHY no ve cable (descartado)

`phystatus 0x84` no tiene el bit `0x02`. En soso es `PHY_LINK_OK`. En Linux 6.6.32 es `LinkStatus = 0x02`. El MAC arrancó (`rtl_hw_start_8168h_1 (VER_46) ok`).

## 4. Orden de corrección

La bisección decide el commit. Hasta tener el resultado de v0.3.22 en placa no se toca `main`.

1. **Arrancar v0.3.22.** Rama `rollback/rog-v0.3.22` en el worktree `../soso-rog-v0.3.22`, commit `4790ea18b` (tag `v0.3.22`, 12 oct). No tiene `HW_POLL`, ni el bombeo de `lx_mdelay`, ni el volcado troceado de `fatlog`. El rootfs empaquetado son 256 MiB; la p2 del stick mide 6 784 MiB. El flasheo lo hace el usuario, desde el worktree: `cargo xtask flash-usb-live /dev/sda --yes --skip-models`. Conserva p3 y `SOSOWIFI.TXT`. En placa: escribir en sosh, `echo prueba > /tmp/x`, `wifi scan`, diez minutos. Hecho si el SOSOLOG siguiente trae `kbd sc>0` y volcados pasados los 94 s, sin error de escritura.
2. **Bisecar si v0.3.22 va bien.** Primero `5f2d77c1d` (29 oct: `HW_POLL` y el HID de puntero). Si falla, el siguiente es `173a5880e` (24 oct: bombeo en `lx_mdelay`). Si va bien, el siguiente es `8c0c5fb57` (7 nov: `fatlog` troceado). Cada paso flashea kernel y rootfs (`--skip-models`), no solo el kernel.
3. **Arreglar en `main` solo el commit que aísle la bisección.** Candidatos, si el síntoma cae ahí: que `poll_keyboard_event` no dependa de `HOSTS` (candado por controlador) y que un trozo de `fatlog` que falla respete el intervalo de 2 s. Host: `cargo xtask check` y `cargo xtask test-usb`. Placa: la misma prueba del paso 1.

Si v0.3.22 también se queda sin teclas, la bisección no se hace: el fallo no está en el código posterior al 12 oct.

## 5. Qué no se ha hecho

`main` no se ha cambiado (el parche sin commitear de `kernel/src/task/mod.rs` sigue ahí). `arranques_consecutivos_ok` sigue en 0. El kernel de `5f2d77c1d` (29 oct, `launcher`) está compilado en el worktree `soso-rog-5f2d77c1d`, rama `rollback/rog-5f2d77c1d`. Aún no está flasheado.

## 6. Arranque de las 09:58 (v0.3.22)

`BOOTMARK` a las 09:58. Copia en `target/usb-diagnostic-2026-11-08-0958/` (no va a git). ESP p1 desmontada. El banner es `soso 0.3.22 (4790ea18b)`. `SOSOHASH` sigue en `886e8bb0d-dirty` del 2026-11-05: el flasheo incremental no reescribe el manifiesto.

| Dato | Valor |
|------|-------|
| Log | flush #52, uptime 212 767 ms. `kbd sc=169 ultimo=0x1c enc=86 ent=86` |
| Userspace | `sosh — escribe 'help' para la ayuda`, luego `$` y `halt: apagando soso` |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí` |
| WiFi | dos `scan fin` (`count=23 end=1` y `count=20 end=1`); `4-way completado, enlace autorizado` |
| DHCP | `net: dhcp 192.168.68.132/24 gw 192.168.68.1` |
| Ethernet | `rtl8169: … enlace DOWN 10M half` (`phystatus 0x84`) |
| Escrituras | no hay `fatlog: aviso`, `usb write` ni panic. El volcado sigue mucho después de los 94 s |

El cuelgue de las 08:24 no se repite. `enc=86 ent=86` dice que cada byte que llegó a la cola se leyó. Una pulsación que no genera informe no entra en `sc` ni en `enc`.

En este arranque se pierden teclas, y ya en el código de octubre:

1. La primera orden se ve como `$ wifisnn` y a continuación corre un `wifi scan` (lista `ssid: dBm` y `SCAN_REQ_UMAC` uid=0x2). En el eco faltan el espacio y `ca`, y `n` sale repetida.
2. La orden siguiente es una racha de borrados (`\x08 \x08`) sobre `wiiconn`, `cnn`, `onect` y acaba en `Ruoo`. El kernel ejecutó `wifi connect` con SSID `Rutilo` y sin contraseña (`está protegida`, `sosh: wifi connect: error de E/S`). Esos caracteres no están todos en la línea del log.
3. En la contraseña del segundo intento hay un borrado a mitad de la línea y, sin salto, sigue `lx: iwl_mvm: scan_count=10`. El printk y el eco comparten la línea.
4. Enter vacío en la lista de redes imprime `wifi: selección inválida` (`user/sosh/src/main.rs` del tag, alrededor de la línea 1101). Eso es la shell, no una tecla perdida.

El HID de arranque es una foto de las teclas pulsadas. `process_report` solo emite el cambio respecto al informe anterior (`crates/xhci-nostd/src/hid.rs`). v0.3.22 sondea el teclado en `read_byte` / `has_input` y en la IRQ1 (`kernel/src/drivers/kbd.rs`, `poll_hw`). Durante `wifi_scan` el BSP se queda en la espera del firmware y no vuelve al planificador: la pulsación que baja y sube entre dos sondeos no incrementa `sc`. `poll_keyboard_event` hace `HOSTS.try_lock()`; si un write USB tiene el candado, ese sondeo no ocurre y la tecla ya soltada no vuelve.

`docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` con este arranque (GSP, VRAM, ALIVE, MVM, scan, asociación, DHCP y sesión en `ok`; `arranques_consecutivos_ok=0`).

## 7. Arranque de `5f2d77c1d` — el bootloader no ve el ELF

La pantalla del ROG es el pánico de `bootloader-x86_64-common` 0.11.15, `lib.rs:103`: `ElfFile::new` no encuentra `7f ELF` al principio del fichero que abre (`kernel-x86_64`). No llega a soso. No dice nada del teclado.

En el host, `soso-rog-5f2d77c1d/target/soso-uefi.img` tiene `KERNEL~1` de 38 556 096 bytes, idéntico a `target/kernel/x86_64-soso/debug/kernel`, y empieza por `7f ELF`. El flasheo dijo `kernel-x86_64 38556096 B → hueco 67108864 B`.

En la ESP, `kernel-x86_64` ocupa el hueco de 64 MiB. Desde el byte 178 688 el contenido coincidía con ese ELF; los 349 sectores anteriores no. Ahí no había `7f ELF`. La raíz tiene además entradas con nombre binario y tamaño de gigabytes; leerlas devuelve error de E/S. `kernel-x86_64`, los `SOSO*.TXT` y `efi/` sí se leen. `BOOTMARK` de las 10:25 dice que el shim cargó `bootsoso.efi` (156 672 bytes) y saltó al bootloader.

Se reescribió `kernel-x86_64` in situ con el ELF del worktree, sin cambiar el tamaño del hueco. Tras desmontar y volver a montar, los 38 556 096 bytes coinciden (`7f ELF`, sha256 `34394dd5af3ae0230c5ead4a9a534082c4fdad485415328df10f6b6371a144b7`). ESP desmontada.

## 8. Arranque de las 10:55 (`5f2d77c1d`) — teclas perdidas y respuesta lenta

`BOOTMARK` a las 10:55. Copia en `target/usb-diagnostic-2026-11-08-1055/` (no va a git). ESP p1 desmontada. Banner `soso 0.3.26 (5f2d77c1d-dirty)`. El `-dirty` son los paquetes de prueba del worktree y los enlaces `lxdde/linux` y `lxdde/reference`, no un parche del kernel. `SOSOHASH` sigue en `886e8bb0d-dirty` del 2026-11-05.

Linux comparado: `lxdde/linux` 6.6.32 (`drivers/usb/host/xhci-ring.c`, `drivers/hid/usbhid/hid-core.c`).

| Dato | Valor |
|------|-------|
| Log | flush #40, uptime 208 647 ms. `kbd sc=131 ultimo=0x1c enc=63 ent=63` |
| Userspace | `sosh`, luego `ls`, `echo "as" > pepe.txt`, `cat pepe.txt` imprime `as`, `halt` |
| Teclado | el eco de la primera orden es `$ widi`, tres borrados, `fi sc`, un borrado, `an`. La línea que se ejecuta es `wifi scan` |
| WiFi | autoconnect: `scan fin` count=21 y 24 `end=1`, `4-way completado`, `net: dhcp 192.168.68.132/24` |
| Scan de la shell | `scan fin count=7 end=3`. `wifi: sondeo parado 16 veces en 30000 ms (max 8067 ms, causa=transporte iwlwifi ocupado)` |
| Otra parada | `sondeo parado 17 veces en 30000 ms (max 9217 ms, causa=volcado del log ~1480 ms)`, junto al `echo` |
| GPU | `GSP_INIT_DONE` |
| Ethernet | `rtl8169` enlace DOWN (`phystatus 0x84`) |

`enc=63 ent=63` dice que cada byte encolado se leyó. La tecla que no genera informe no entra en la cuenta.

### Hallazgos

**H5 — el scan deja el teclado sin sondeo varios segundos (confirmado).** `wifi::scan` coge `TRANS_LOCK` durante toda la llamada (`kernel/src/lxdde/wifi.rs:423-425` en `5f2d77c1d`). `wifi::poll` hace `try_lock` y, si no puede, sale sin actualizar el reloj (`wifi.rs:312-314`). El aviso de 8067 ms sale con `causa=transporte iwlwifi ocupado`, que es justo el contador de esos `try_lock` fallidos (`wifi.rs:143-145`). En esa espera, `lx_mdelay` en la BSP solo llama a `net::poll` y `wifi::poll` (`kernel/src/lxdde/timer.rs:71-75`). No llama al teclado. Linux entrega el HID por interrupción: `xhci_irq` (`drivers/usb/host/xhci-ring.c:3093`) drena el anillo y `hid_irq_in` (`drivers/hid/usbhid/hid-core.c:271`) pasa el informe aunque el scan duerma en otro contexto. En soso no hay IRQ de xHCI: si la BSP no sondea, la pulsación que baja y sube desaparece, y la que sigue pulsada aparece cuando el scan suelta el candado. Eso es la tardanza.

**H6 — el sondeo HID sigue pidiendo el candado de los discos USB (confirmado en el código, compatible con el eco).** `poll_keyboard_event` hace `HOSTS.try_lock()` (`kernel/src/drivers/usb_storage.rs:403`). El comentario dice que no hace falta el candado del vector. El teclado está en el xHCI `06:00.3` y el stick en `06:00.4`. Un write BOT (fatlog, `echo` a sosofs) hace fallar el `try_lock`. El volcado que el log mide en ~1480 ms cae en la misma ventana que la parada de 9217 ms. En Linux `hid_irq_in` no toma el candado del almacenamiento. El eco `$ widi` con tres borrados hasta dejar `wifi scan` es de antes de que el scan devuelva: teclas que hubo que borrar, en el prompt.

**H7 — `HW_POLL` es un candado que espera (hipótesis de tardanza extra).** `poll_hw` hace `HW_POLL.lock()` (`kernel/src/drivers/kbd.rs:648` en este commit) y dentro sondea el xHCI. La IRQ1 y la lectura de la tty se quedan detrás de ese sondeo. El log no aísla este candado de H5 y H6.

### Orden de corrección

1. En la espera del scan, sondear el HID sin `TRANS_LOCK`. `bombeo_red_bsp` en `kernel/src/lxdde/timer.rs` debe llamar a `kbd::poll_devices` (o el sondeo USB del teclado) además de la red. `wifi::poll` no sirve ahí: el `try_lock` de `TRANS_LOCK` falla durante todo el scan. Hecho en placa: durante `wifi scan`, `kbd sc` sube y no aparece `sondeo parado` por encima de 500 ms. En host, un test con `TRANS_LOCK` cogido sigue entregando un informe HID fingido.
2. Sacar `HOSTS.try_lock()` de `poll_keyboard_event` (`kernel/src/drivers/usb_storage.rs`). El xHCI del teclado se sondea con su propio candado. Hecho en placa: teclear mientras `echo` escribe en sosofs y el eco coincide con lo tecleado. En host, el test del sondeo no toma `HOSTS`.
3. No retener `HW_POLL` durante el sondeo USB largo (`kernel/src/drivers/kbd.rs`). Un `try_lock` que falle no puede tirar un evento ya sacado del anillo. Hecho en placa: la segunda tecla de una ráfaga no espera a que termine el sondeo de la primera.

### Qué no se ha hecho

El arreglo está en la rama `rollback/rog-5f2d77c1d` y, sin commit, en `main`. `bombeo_red_bsp` sondea el HID sin `TRANS_LOCK`. `poll_keyboard_event` ya no toma `HOSTS`. `HW_POLL` solo cubre el i8042, y una tecla cuyo candado falla vuelve a la cola. El xHCI con HID arma MSI-X (o INTx) y el handler vacía el anillo como `xhci_irq`. El kernel de ese worktree compiló. No se ha flasheado. `arranques_consecutivos_ok` sigue en 0. `docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` siguen con el arranque de las 10:55.
