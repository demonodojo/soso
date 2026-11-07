# Diagnóstico ROG — 2026-11-07

El live llega a `sosh` y se queda en el selector de redes. La GPU completa GSP, el pool de VRAM y el CE. El clear 3D consume el pushbuffer y el semáforo sigue en 0. El AX200 hace ALIVE y lista 18 redes. Nadie pulsó una tecla (`kbd sc=0`). No hay asociación ni DHCP.

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Pendrive | `/dev/sda`, ESP p1 `KERNEL`. Montada en `/run/media/jmdiez/KERNEL` y desmontada |
| Copia | `target/usb-diagnostic-2026-11-07/` (no va a git) |
| Log | flush #21, uptime 97 187 ms, 56 205 bytes útiles. `kbd sc=0 ultimo=0x00 enc=0 ent=0` |
| Kernel | `soso 0.3.28 (886e8bb0d-dirty)`, empaquetado 2026-11-05 (`SOSOHASH.TXT`) |
| Rootfs | sosofs generación 36, 1 736 704 bloques; modelos `qwen27-iq3` |
| PCI | `10de:249c` GA104 (RTX 3080 Laptop, 16 384 MiB), `8086:2723` AX200, `10ec:8168` rtl8169 |
| Shim | `BOOTMARK` (7 nov 15:10): `soso-shim: UEFI alcanzado`, `bootsoso.efi` 156 672 bytes |
| Linux | `lxdde/linux` 6.6.32 (`nvkm/engine/fifo/ga100.c`, `drivers/usb/core/hub.c`, `r8169_main.c`) |
| Clase 3D | OGKM `open-gpu-kernel-modules-570.144` (tag `570.144`), `clc797.h` (la clase del log es `0xc797`) |

Hostchecks (no cubren DMA Falcon, el clear en silicio ni un scan real):
`./scripts/l6-iwl-fw-hostcheck.sh` y `./scripts/l6-g3-gsp-hostcheck.sh` en
verde. Salida en `target/usb-diagnostic-2026-11-07/hostcheck-iwl.txt` y
`hostcheck-gsp.txt`. El hostcheck GSP imprime `OK: pushbuffer 3D CLEAR_SURFACE
(cls Blackwell 0xce97, semáforo 0xf010)` sobre el árbol de trabajo. El kernel
de este USB es el de HEAD, donde ese dword sigue siendo `0x10`.

No se valida el parche en placa. No se ha flasheado.

## 2. Etapas

| Etapa | Cita del SOSOLOG | Resultado |
|-------|------------------|-----------|
| shim | `BOOTMARK`: `bootsoso.efi leído (156672 bytes)` | ok |
| userspace | `sosh — escribe 'help' para la ayuda`; flush #21 a los 97 s | ok |
| GPU GSP | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `GSP=rm_compute, pool VRAM=sí` | ok |
| CE | `CE readback verificado (G4e GO)`; tras el clear, `sonda CE — … tras PROMOTE_CTX=mueve` | ok |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `3D clear falló en autoprueba (compute sigue)` | fallo |
| WiFi | `firmware ALIVE (UCODE_ALIVE_NTFY)`; `alive=true`; `up mínimo listo`; `SCAN_COMPLETE status=1 … end=1` | ok hasta el scan |
| Asociación | `wifi: SOSOWIFI.TXT sin ssid= válido`; el log acaba en `wifi: número de red (Enter para omitir):` | no se intentó |
| DHCP | no hay `net: dhcp`. `rtl8169: … enlace DOWN 10M half` | pendiente |
| Ethernet | `rtl8169: rtl_hw_start_8168h_1 (VER_46) ok`; enlace DOWN | driver arriba, sin cable |

## 3. Hallazgos

### H1 — Enter en el selector de redes no omite (confirmado en el binario; este arranque no lo pulsó)

El prompt dice que Enter omite. Este flush no llega a `wifi: selección inválida`:
la cabecera trae `kbd sc=0`. El mismo `SOSOHASH` (`886e8bb0d-dirty`, 2026-11-05)
sí lo imprimió el 2026-11-06, con `ultimo=0x1c` y una línea vacía.

En HEAD, `Lector::siguiente` devuelve `Ok(Some(""))` en una línea vacía
(`user/libsoso/src/linea.rs:148-159`). `Ok(None)` está reservado al Ctrl-D.
El selector manda cualquier `Some`, incluida la cadena vacía, a `parse::<usize>()`
(`user/sosh/src/main.rs:1131` en `886e8bb0d`). La cadena vacía no es un índice
y sale `selección inválida`.

El árbol de trabajo, sin commitear, ya vuelve si `eleccion.trim()` está vacío
(`user/sosh/src/main.rs:1131-1135`). Ese `sosh` no está en el rootfs de este USB.
`--only kernel` no lo actualiza: vive en el rootfs.

No es un fallo del driver. El AX200 llegó a `UCODE_ALIVE_NTFY`,
`SCAN_REQ_UMAC v15` y `SCAN_COMPLETE end=1` con `count=22` y 18 BSS.

### H2 — el clear 3D programa el semáforo con `PIPELINE_LOCATION_NONE` (confirmado)

La clase reservada es `0xc797`. El pushbuffer se consume: `gpput=1`,
`USERD GPGet=1 GPPut=1`, y `Get`/`Put` quedan en `537035424` (`0x200282A0`),
el final de los 40 dwords que salen de `0x8020028200`. `PBDMA0 INTR_0=0`.
El semáforo sigue en 0 a los 5 s.

En HEAD, `GSP_3D_REPORT_RELEASE_WRITES` es `0x10`
(`lxdde/ports/nouveau/gsp_3d.h:41` del commit `886e8bb0d`). Ese valor es el
que escribe `gsp_3d.c:116-117` en `SET_REPORT_SEMAPHORE_D` antes de
`CLEAR_SURFACE`.

En `clc797.h` del OGKM 570.144 el mismo método parte así:

- bits 1:0 `OPERATION`: `RELEASE` = 0 (`clc797.h:3923-3924`)
- bit 4 `RELEASE`: `AFTER_ALL_PRECEEDING_WRITES_COMPLETE` = 1 (`clc797.h:3928-3930`)
- bits 15:12 `PIPELINE_LOCATION`: `NONE` = 0, `ALL` = `0xF` (`clc797.h:3934-3946`)

`0x10` es RELEASE con las escrituras precedentes y la etapa a cero. Una
liberación en `NONE` no tiene etapa de la que retirarse: el PBDMA se bebe
el pushbuffer y el semáforo no pasa a 1. El árbol de trabajo ya cambia el
define a `0x10 | (0xF << 12)` (`gsp_3d.h:45`) y el hostcheck GSP lo exige.
Ese dword no iba en el kernel de este USB.

Descartado como causa: el volcado `RAMFC … NO_COINCIDE` y
`canal GR — inst+0x200 sin directorio (0x0)`. `ga100_chan_ramfc_write`
(`lxdde/linux` 6.6.32, `fifo/ga100.c:74-77`) escribe `0x0000face` en `+0x010`
y la base del GPFIFO en `+0x048`. En el log esos offsets, y los 0x200 bytes
siguientes, son cero. El USERD de otra dirección sí avanza (`GPGet=1`), así
que esa imagen PRAMIN no es el RAMFC que usó el motor.

Descartado también el atasco del CE. Las sondas de después, incluida
`tras PROMOTE_CTX`, imprimen `mueve`.

### H3 — el enlace ethernet está caído porque el PHY no ve cable (descartado)

`phystatus 0x84` no tiene el bit `0x02`. En soso ese bit es `PHY_LINK_OK`
(`kernel/src/drivers/rtl8169.rs:112` y `:1177`). En Linux 6.6.32 es
`LinkStatus = 0x02` (`r8169_main.c:457`). El BMSR `0x7989` tampoco tiene el
bit 2 de link. El MAC arrancó (`rtl_hw_start_8168h_1 (VER_46) ok`). No hay
cambio de código.

### H4 — el new scheme USB2 babea y el esquema viejo enumera (descartado)

`GET_DESCRIPTOR(type=1)` devuelve `code=3` (Babble) tres veces en los teclados
`0b05:1866` y `0b05:18c6`, y el código cae a «old scheme». Los dos quedan
`keyboard ready`. El Bluetooth `8087:0029` falla un Address Device con
`code=4` y el reintento lo direcciona. Linux 6.6.32 hace el GET de 64 B en
el new scheme y, si falla, sigue el bucle (`hub.c:4960-4992`). El mass storage
del live sale con `1048576000` sectores y `live: GPT backend=Usb`. No bloquea
el arranque.

## 4. Orden de corrección

Los dos cambios ya están en el árbol de trabajo, sin commitear y sin flashear.
Este USB no los ejecuta.

1. **Enter vacío omite la red.** `user/sosh/src/main.rs`, `wifi_setup_interactivo`.
   Si `eleccion.trim()` está vacío, volver igual que con `Ok(None)`, sin imprimir
   `selección inválida`. No cambiar `Lector`: una línea vacía y Ctrl-D no son
   lo mismo. El árbol ya lo hace en las líneas 1131-1135. Host: una línea vacía
   no llega a `parse`. Placa: Enter en `número de red (Enter para omitir)` deja
   `$` y no escribe `selección inválida`. Hace falta rootfs, no `--only kernel`.
2. **Semáforo 3D con `PIPELINE_LOCATION_ALL`.** `lxdde/ports/nouveau/gsp_3d.h`
   tiene que valer `0x10 | (0xF << 12)` = `0xF010`: el bit 4 de `clc797.h:3930`
   más `ALL` en los bits 15:12 (`clc797.h:3946`). El árbol ya lo tiene en la
   línea 45 y `./scripts/l6-g3-gsp-hostcheck.sh` imprime `semáforo 0xf010`.
   El kernel `886e8bb0d` de este USB no. Host: el pushbuffer del clear lleva
   `0xF010` en `SET_REPORT_SEMAPHORE_D`. Placa: `3D clear verificado (H1 GO)`,
   o el semáforo llega a 1. Si con `0xF010` sigue en 0, el siguiente paso es
   emitir `CLEAR_SURFACE` antes del release; eso hoy es hipótesis.

Para llevar los dos al stick hace falta un flash con rootfs y kernel
(`cargo xtask flash-usb-live /dev/sda --yes --skip-models`). No lo he lanzado.

## 5. Qué no se ha hecho

En la sesión de la tarde no se tocó el USB. El árbol ya tenía los dos parches
sin commitear. `./scripts/l6-g3-gsp-hostcheck.sh` y
`./scripts/l6-iwl-fw-hostcheck.sh` cierran en verde y no cubren el clear en
silicio ni un scan real.

`docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` con este arranque (GSP, VRAM,
CE y MVM en `ok`; scan en `ok` con 18 BSS; asociación y DHCP en `pendiente`;
`sesion_sostenida=ok` por sosh a los 97 s; `arranques_consecutivos_ok=0`).

## 6. Arranque de las 19:46

Mismo stick, kernel distinto. `BOOTMARK` a las 19:46. Copia en
`target/usb-diagnostic-2026-11-07-1946/` (no va a git). ESP p1 desmontada.
`SOSOHASH` sigue en `886e8bb0d-dirty` del 2026-11-05: el kernel se actualizó
in situ y el manifiesto no.

| Dato | Valor |
|------|-------|
| Log | flush #20, uptime 114 232 ms. `kbd sc=3 ultimo=0x06 enc=1 ent=1` |
| Banner | `soso 0.3.28 (8c0c5fb57)`, sin `-dirty` |
| Userspace | `sosh — escribe 'help' para la ayuda` |
| GPU | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `pool VRAM=sí`; `CE readback verificado (G4e GO)` |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `USERD GPGet=1 GPPut=1`; 40 dwords; `PBDMA0 INTR_0=0`; `3D clear falló en autoprueba` |
| WiFi | `UCODE_ALIVE_NTFY`; `up mínimo listo`; `SCAN_COMPLETE status=1 uid=0x1 count=23 end=1`; 19 BSS |
| Selector | `wifi: número de red (Enter para omitir): 5` y el log acaba ahí |
| Ethernet | `rtl8169: … enlace DOWN 10M half` (`phystatus 0x84`) |

`8c0c5fb57` ya define `GSP_3D_REPORT_RELEASE_WRITES` como `0x10 | (0xF << 12)`.
El semáforo sigue en 0. `SET_REPORT_SEMAPHORE_D` (`clc797.h:3922`) es el método
que suelta el release y espera a las escrituras precedentes (`clc797.h:3928-3930`,
etapa `ALL` en `clc797.h:3934-3946`). En el pushbuffer de ese kernel el clear
iba detrás, así que no era una escritura precedente.

En el árbol, sin flashear, `encode_clear_pb` emite `CLEAR_SURFACE` (`0x19d0`)
antes de ese dword. `./scripts/l6-g3-gsp-hostcheck.sh` imprime `semáforo 0xf010`
y exige que el método `0x19d0` vaya delante. No cubre el clear en silicio.
No se ha puesto el bit 28 `STRUCTURE_SIZE_ONE_WORD`.

`docs/hw-matrix.json`: las dos entradas con este arranque (GSP, VRAM, CE, ALIVE,
MVM y scan en `ok`; asociación y DHCP en `pendiente`; `arranques_consecutivos_ok=0`).

## 7. Arranque de las 20:17

`BOOTMARK` a las 20:17. Copia en `target/usb-diagnostic-2026-11-07-2024/`
(no va a git). ESP p1 desmontada. `SOSOHASH` sigue en `886e8bb0d-dirty` del
2026-11-05.

| Dato | Valor |
|------|-------|
| Log | flush #19, uptime 96 528 ms. Es el último volcado; fatlog sondea cada 2 s |
| Banner | `soso 0.3.28 (8c0c5fb57-dirty)` |
| Userspace | `sosh — escribe 'help' para la ayuda`, luego `wifi: escaneando redes…` |
| GPU | `GSP_INIT_DONE`; `pool VRAM=sí`; `CE readback verificado (G4e GO)` |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `USERD GPGet=1 GPPut=1`; 40 dwords; `PBDMA0 INTR_0=0` |
| WiFi | `UCODE_ALIVE_NTFY`; `SCAN_REQ_UMAC v15`; el log acaba en `scan_count=15`. No hay `scan fin` |
| Teclado | `kbd sc=0` |
| Ethernet | `rtl8169: … enlace DOWN 10M half` (`phystatus 0x84`) |

El semáforo sigue en 0 con el clear ya delante del release: no es este cuelgue.
`3D clear falló en autoprueba (compute sigue)` sale y el arranque llega a sosh.
Volver al encode de octubre (`0x10`, clear después del release, commit
`79a57a89a`) restituye un dword que ya falló el mismo día. El 10 de octubre la
shell llegaba y el enlace servía (`docs/DIAGNOSTICO-ROG-2026-10-10.md`); el
clear 3D aún no existía.

El cuelgue es el scan. `sosh` llama a `wifi_setup_si_hace_falta` antes del
prompt y `scan()` no suelta `TRANS_LOCK` hasta que el bucle acaba. Desde el
24 de octubre (`173a5880e`) `lx_mdelay` en la BSP bombea `net::poll` en cada
milisegundo. Linux no hace eso: `iwl_poll_bit` solo lee un registro y hace
`udelay` (`lxdde/linux/.../iwl-io.c`, 6.6.32).

En el árbol, sin flashear, las esperas de iwlwifi que corren con el transporte
cogido usan `iwl_mdelay_locked` (`lx_udelay`, sin bombeo). El bucle del scan
corta a los 8 s por contador o por `lx_ktime_get_ns`, y siempre imprime
`scan fin`. `./scripts/l6-iwl-fw-hostcheck.sh` cierra en verde, con
`scan cuyo poll no progresa sale por timeout`. No cubre un scan en silicio.

`docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` con este arranque (GSP, VRAM,
CE, ALIVE y MVM en `ok`; scan en `fail`; asociación y DHCP en `pendiente`;
`arranques_consecutivos_ok=0`).
