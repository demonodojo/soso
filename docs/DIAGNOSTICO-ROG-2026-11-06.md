# Diagnóstico ROG — 2026-11-06

El live llega a `sosh` y se queda ahí 37 minutos. La GPU completa GSP, el pool
de VRAM y el CE. El clear 3D no suelta el semáforo. El WiFi hace ALIVE y
encuentra 18 redes; no hay asociación porque `SOSOWIFI.TXT` no trae `ssid=`
y Enter en el selector se trata como una elección inválida.

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Pendrive | `/dev/sda`, ESP p1 `KERNEL` montada en `/run/media/jmdiez/KERNEL` y desmontada |
| Copia | `target/usb-diagnostic-2026-11-06/` (no va a git) |
| Log | flush #23, uptime 2 257 744 ms, 56 069 bytes útiles. `kbd sc=2 ultimo=0x1c enc=1 ent=1` |
| Kernel | `soso 0.3.28 (886e8bb0d-dirty)`, empaquetado 2026-11-05 (`SOSOHASH.TXT`) |
| Rootfs | sosofs generación 36, 1 736 704 bloques (6 784 MiB); modelos `qwen27-iq3` |
| PCI | `10de:249c` GA104 (RTX 3080 Laptop, 16 384 MiB), `8086:2723` AX200, `10ec:8168` rtl8169 |
| Shim | `BOOTMARK`: `soso-shim: UEFI alcanzado`, `bootsoso.efi` 156 672 bytes |
| Linux | `lxdde/linux` 6.6.32 (`nvkm/engine/fifo/ga100.c`, solo para descartar el volcado RAMFC) |
| Clase 3D | OGKM `open-gpu-kernel-modules-570.144`, `clc797.h` (la clase del log es `0xc797`) |

Hostchecks (no cubren DMA Falcon, el clear en silicio ni un scan real):
`./scripts/l6-iwl-fw-hostcheck.sh` y `./scripts/l6-g3-gsp-hostcheck.sh` en
verde. Salida en `target/usb-diagnostic-2026-11-06/hostcheck.txt`. El
hostcheck GSP ya exige `PIPELINE_LOCATION_ALL` sobre el árbol de trabajo;
el kernel de este USB es el de HEAD, donde ese dword sigue siendo `0x10`.

No se valida: ni el parche en placa ni un segundo arranque. No se ha flasheado.

## 2. Etapas

| Etapa | Cita del SOSOLOG | Resultado |
|-------|------------------|-----------|
| shim | `BOOTMARK`: `bootsoso.efi leído (156672 bytes)` | ok |
| userspace | `sosh — escribe 'help' para la ayuda`; flush #23 a los 37 min | ok |
| GPU GSP | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `GSP=rm_compute, pool VRAM=sí` | ok |
| CE | `CE readback verificado (G4e GO)`; tras el clear, `sonda CE — … tras PROMOTE_CTX=mueve` | ok |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `3D clear falló en autoprueba (compute sigue)` | fallo |
| WiFi | `firmware ALIVE (UCODE_ALIVE_NTFY)`; `alive=true`; `SCAN_COMPLETE status=1 … end=1` | ok hasta el scan |
| Asociación | `wifi: SOSOWIFI.TXT sin ssid= válido`; `wifi: selección inválida` | no se intentó |
| DHCP | no hay `net: dhcp`. `rtl8169: … enlace DOWN 10M half` | pendiente |
| Ethernet | `rtl8169: rtl_hw_start_8168h_1 (VER_46) ok`; enlace DOWN | driver arriba, sin cable |

## 3. Hallazgos

### H1 — Enter en el selector de redes no omite (confirmado)

El prompt dice que Enter omite. El log, justo después, es un salto de línea
y `wifi: selección inválida`, y luego `$`. No hay ningún dígito entre el
prompt y el aviso. La cabecera del flush trae un solo Enter (`ultimo=0x1c`,
`ent=1`).

`Lector::siguiente` devuelve `Ok(Some(""))` en una línea vacía
([user/libsoso/src/linea.rs](../user/libsoso/src/linea.rs):148-159). `Ok(None)`
está reservado al Ctrl-D. El selector solo trata `None` como omitir y manda
cualquier otra cosa, incluida la cadena vacía, a `parse::<usize>()`
([user/sosh/src/main.rs](../user/sosh/src/main.rs):1122-1136). La cadena vacía
no es un índice y sale `selección inválida`.

No es un fallo del driver. En este arranque el AX200 llegó a
`UCODE_ALIVE_NTFY`, `SCAN_REQ_UMAC v15` y `SCAN_COMPLETE end=1` con 18 BSS.
No hay discrepancia de transporte que explicar: el scan terminó.

### H2 — el clear 3D programa el semáforo con `PIPELINE_LOCATION_NONE` (confirmado)

La clase reservada es `0xc797`. El pushbuffer se consume: `gpput=1`,
`USERD GPGet=1 GPPut=1`, y `Get`/`Put` quedan al final de los 40 dwords
(`0x200282A0`). `PBDMA INTR_0=0`. El semáforo sigue en 0 a los 5 s.

En HEAD, `GSP_3D_REPORT_RELEASE_WRITES` es `0x10`
(`lxdde/ports/nouveau/gsp_3d.h`, línea 41 del commit `886e8bb0d`). Ese valor
es el que escribió `gsp_3d.c` en `SET_REPORT_SEMAPHORE_D` antes de
`CLEAR_SURFACE` (`gsp_3d.c:116-120`).

En `clc797.h` del OGKM 570.144 el mismo método parte así:

- bits 1:0 `OPERATION`: `RELEASE` = 0 (`clc797.h:3923-3924`)
- bit 4 `RELEASE`: `AFTER_ALL_PRECEEDING_WRITES_COMPLETE` = 1 (`clc797.h:3928-3930`)
- bits 15:12 `PIPELINE_LOCATION`: `NONE` = 0, `ALL` = `0xF` (`clc797.h:3934-3946`)

`0x10` es RELEASE con las escrituras precedentes y la etapa a cero. Una
liberación en `NONE` no tiene etapa de la que retirarse: el PBDMA se bebe
el pushbuffer y el semáforo no pasa a 1. El árbol de trabajo, sin commitear,
ya cambia el define a `0x10 | (0xF << 12)` (`gsp_3d.h:45`) y el hostcheck GSP
lo exige. Ese dword no iba en el kernel de este USB.

Descartado como causa: el volcado `RAMFC … NO_COINCIDE` y
`canal GR — inst+0x200 sin directorio (0x0)`. `ga100_chan_ramfc_write`
(`lxdde/linux` 6.6.32, `fifo/ga100.c:74-77`) escribe `0x0000face` en `+0x010`
y la base del GPFIFO en `+0x048`. En el log esos offsets, y los 0x200 bytes
siguientes, son cero. El USERD de otra dirección sí avanza (`GPGet=1`), así
que esa imagen PRAMIN no es el RAMFC que usó el motor. `chan_join_gr_pdb`
([gsp_chan.c](../lxdde/ports/nouveau/gsp_chan.c):349-352) abandona al leer
ese mismo cero. No explica un pushbuffer ya consumido.

Descartado también el atasco del CE por runlist compartida. En esta GA104,
GR0 y CE0 salen los dos con runlist `0xc00000`, pero las cuatro sondas de
después, incluida `tras PROMOTE_CTX`, imprimen `mueve`. El canal de GR llegó
a `GPGet==GPPut` y no retuvo la runlist. El síntoma de la GB205 de hoy (el
CE deja de señalizar) no está en este log.

### H3 — dos líneas del listado de redes pierden caracteres (hipótesis)

En el binario del log, la red 4 queda `Rutilo53 dBm, canal 3, WPA)` (faltan
` (-`) y la 13 empieza en `. vodafoneD210`. Las otras dieciséis, incluidos
otros `Rutilo`, salen bien. El formato de sosh es
`  {}. {} ({} dBm, canal {}, {sec})`. La consola borra con `0x08` y un `\r`
vacía la fila ([kernel/src/drivers/fb.rs](../kernel/src/drivers/fb.rs):829 y
843), y el fatlog guarda el texto ya procesado. No está demostrado que el
SSID trajera esos bytes: no aparecen en el hueco. No bloqueó la elección:
el usuario pulsó Enter, no un número.

## 4. Orden de corrección

1. **Enter vacío omite la red.** [user/sosh/src/main.rs](../user/sosh/src/main.rs)
   `wifi_setup_interactivo`, en el `match` de la línea 1131. Si
   `eleccion.trim()` está vacío, volver igual que con `Ok(None)`, sin imprimir
   `selección inválida`. No cambiar `Lector`: una línea vacía y Ctrl-D no son
   lo mismo. Host: una línea vacía no llega a `parse`. Placa: Enter en
   `número de red (Enter para omitir)` deja `$` y no escribe
   `selección inválida`.
2. **Semáforo 3D con `PIPELINE_LOCATION_ALL`.** El define de
   [gsp_3d.h](../lxdde/ports/nouveau/gsp_3d.h) tiene que valer
   `0x10 | (0xF << 12)` = `0xF010`, que es el bit 4 de
   `clc797.h:3930` más `ALL` en los bits 15:12 (`clc797.h:3946`). El árbol de
   trabajo ya lo tiene y `./scripts/l6-g3-gsp-hostcheck.sh` lo comprueba; el
   kernel `886e8bb0d` de este USB no. No hace falta mover el CE a otra
   runlist: aquí sigue copiando después del clear. Host: el pushbuffer del
   clear lleva `0xF010` en `SET_REPORT_SEMAPHORE_D`. Placa: `3D clear
   verificado (H1 GO)`, o el semáforo llega a 1. Si con `0xF010` sigue en 0,
   el siguiente paso es emitir `CLEAR_SURFACE` antes del release para que el
   clear sea una escritura precedente; eso hoy es hipótesis.

## 5. Qué no se ha hecho

El USB sigue con el kernel y el rootfs del 2026-11-05. En el árbol de trabajo,
sin commitear y sin flashear, están los dos cambios: `GSP_3D_REPORT_RELEASE_WRITES`
vale `0x10 | (0xF << 12)` y `wifi_setup_interactivo` vuelve si la línea, tras
`trim`, está vacía. `./scripts/l6-g3-gsp-hostcheck.sh` de esta sesión imprime
`OK: pushbuffer 3D CLEAR_SURFACE (cls Blackwell 0xce97, semáforo 0xf010)` y
cierra con `OK: banco GSP sin hallazgos de ASan/UBSan`. Eso no cubre el clear
en silicio ni un scan real. No se ha reordenado `CLEAR_SURFACE`: el pushbuffer
de este USB ya se consumió con el semáforo en 0, y ese síntoma lo explica
`PIPELINE_LOCATION_NONE`.

`docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` con los dos arranques del
día (GSP, VRAM y CE en `ok`; 3D fuera de etapa; ALIVE, MVM y scan en `ok`;
asociación y DHCP en `pendiente`; `sesion_sostenida=ok` por sosh;
`arranques_consecutivos_ok=0`).

## 6. Segundo arranque (18:31)

Mismo stick, mismo kernel `soso 0.3.28 (886e8bb0d-dirty)`. Copia en
`target/usb-diagnostic-2026-11-06-1831/` (no va a git). ESP p1 desmontada
tras copiar. No es una prueba del parche: el USB no lo lleva.

| Dato | Valor |
|------|-------|
| Log | flush #21, uptime 100 079 ms, 55 986 bytes útiles. `kbd sc=4 ultimo=0x1c enc=2 ent=2` |
| Rootfs | sosofs generación 57, 1 736 704 bloques |
| Shim | `BOOTMARK`: `soso-shim: UEFI alcanzado`, `bootsoso.efi` 156 672 bytes |
| Userspace | `sosh — escribe 'help' para la ayuda` |
| GPU | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `pool VRAM=sí`; `CE readback verificado (G4e GO)`; tras el clear, `tras PROMOTE_CTX=mueve` |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `USERD GPGet=1 GPPut=1`; `PBDMA0 INTR_0=00000000`; `3D clear falló en autoprueba (compute sigue)` |
| WiFi | `UCODE_ALIVE_NTFY`; `alive=true`; `SCAN_REQ_UMAC v15`; `SCAN_COMPLETE status=1 uid=0x1 count=19 end=1`; 16 BSS |
| Asociación | `wifi: SOSOWIFI.TXT sin ssid= válido`; `wifi: selección inválida` |
| Ethernet | `rtl8169: … enlace DOWN 10M half` |

`wifi: sondeo parado 3758 ms (… causa=transporte iwlwifi ocupado)` sale mientras
el scan sigue contando y termina con `end=1`. No es un fallo de transporte.

El listado de las 16 redes sale entero. La hipótesis H3 del flush #23 (dos
líneas con caracteres perdidos) no se repite aquí.
