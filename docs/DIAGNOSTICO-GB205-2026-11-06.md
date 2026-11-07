# Diagnóstico GB205 — 2026-11-06

El live arranca el GSP y el CE, el clear 3D no señaliza y deja el CE atascado,
y el kernel no llega a `boot: gpu` ni a sosh: el log se corta en el probe del
AX211, antes de que `start_firmware` escriba nada.

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Pendrive | `/dev/sda`, ESP p1 `KERNEL` (`4F7E-3B75`), copiada y desmontada |
| Logs | `target/usb-diagnostic-2026-11-06/`, flush #10, uptime 74284 ms, 71414 bytes |
| Kernel | `soso 0.3.28 (886e8bb0d-dirty)`; `SOSOHASH` misma versión y build, fecha 2026-11-05T09:57:34+01:00 |
| Rootfs | sosofs generación 51, 1 736 704 bloques (6784 MiB); modelos `qwen27-iq3` + `tiny` |
| PCI | GPU `10de:2f18` (RTX 5070 Ti Laptop), WiFi `8086:7f70` (AX211), ethernet `10ec:8125`, NVMe `1344:5425` |
| Shim | `BOOTMARK`: UEFI AMI 2.90, `bootsoso.efi` 156672 bytes |
| Linux iwlwifi / r8169 | `lxdde/linux/` 6.6.32 |
| Linux nouveau | `lxdde/reference/linux-master-nouveau/` `fc02acf` |
| Offsets de clase 3D | `open-gpu-kernel-modules-570.144` `clc597.h` |

Hostchecks del mismo árbol, en verde, y no cubren esto: `l6-iwl-fw-hostcheck.sh`
y `l6-g3-gsp-hostcheck.sh` (salida en
`target/usb-diagnostic-2026-11-06/hostcheck.txt`). No ejercitan el reloj de
`iwl_poll_bit`, el transporte PCIe ni un clear 3D en silicio.

No se ha tocado código ni se ha reflasheado. No hay arranque nuevo de placa.

## 2. Etapas

| Etapa | Cita del SOSOLOG | Resultado |
|-------|------------------|-----------|
| shim | `BOOTMARK`: `bootsoso.efi leído (156672 bytes)` | ok |
| live / fs | `live: GPT backend=Usb root LBA 393250 (6784 MiB)`; `fs: sosofs live (generación 51, 1736704 bloques)` | ok |
| GPU firmware | `GSP_INIT_DONE recibido tras 22 mensaje(s)`; `GSP booted (hw, GSP-FMC vía FSP, 12227 MiB VRAM)` | ok |
| CE | `CE readback verificado (G4e GO)` y tres sondas `mueve 4 KiB`; luego `CE — semáforo no llegó a 12 en 2000 ms (vale 11)` y `CE atascado` | fallo al final |
| 3D | `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`; `3D clear falló en autoprueba` | fallo |
| WiFi | `iwlwifi: probe id=0x7f70 … gen3`; `iwlwifi probe 8086:7f70 ok`. No hay `UCODE_ALIVE_NTFY` ni `iwlwifi start` | no arrancó |
| userspace | último `boot:` es `boot: ethernet`. No hay `boot: gpu`, `boot: red`, `boot: task` ni `sosh` | no llegó |

## 3. Hallazgos

### H1 — el arranque se queda en el reloj del MAC del AX211 (confirmado el desajuste; el corte del log encaja)

Tras `lxdde-pci: iwlwifi probe 8086:7f70 ok` no hay ni una línea más. En
`kernel/src/lxdde/mod.rs` lo siguiente es `wifi::start_firmware()`, que llama a
`iwl_trans_gen3_start`. La primera espera larga y muda es
`iwl_finish_nic_init`: sondea `MAC_CLOCK_READY` 25000 veces.

- soso, `lxdde/ports/iwlwifi/iwl_trans.c:61-68`: cada vuelta es `lx_mdelay(1)`.
  25000 vueltas son 25 s. El comentario de `iwl_finish_nic_init` (línea 392)
  dice «25 s».
- Linux 6.6.32, `drivers/net/wireless/intel/iwlwifi/iwl-io.c:48-60`:
  `IWL_POLL_INTERVAL` es 10 µs y el tope es el cuarto argumento en
  microsegundos. `iwl_finish_nic_init` (`iwl-io.c:480`) pasa 25000, o sea
  25 ms. La familia del AX211 en ese árbol es `IWL_DEVICE_FAMILY_AX210`
  (`cfg/ax210.c`, `iwl_so_trans_cfg`), el mismo camino `INIT_DONE` que usa
  soso, no el de BZ.

El SOSOLOG no trae `MAC no despertó` ni `context-info gen3`. Esas líneas se
imprimen al salir de la espera o al terminar de subir el firmware. Con el
último flush en el probe, el kernel estaba dentro de ese tramo mudo. Si el bit
no llega, Linux falla en 25 ms y sigue; soso se queda callado 25 s y el
usuario ve la pantalla parada justo después del volcado de la GPU.

`fatlog` no reescribe la cabecera si el anillo no crece, así que un cuelgue
mudo no sube el uptime del flush. No queda demostrado que el bit no llegara:
solo que no se imprimió nada después del probe.

### H2 — el clear 3D no señaliza y atasca el CE0, que comparte runlist con GR0 (confirmado)

Orden en el log, después de `CE readback verificado (G4e GO)`:

1. Sondas CE antes de GR0, tras el golden y tras crear GR0: `mueve 4 KiB`.
2. Promoción del contexto de usuario: `contexto de GR promocionado: 8 entradas` (el RPC no se rechazó).
3. `RM_ALLOC` de compute `0xcec0` y de 3D `0xce97`: ok.
4. `3D — semáforo no llegó a 1 en 5000 ms (vale 0)`, `GPPut=1`, `Put=672`.
5. `CE — semáforo no llegó a 12 en 2000 ms (vale 11)` y `CE atascado`.
6. `sonda CE — … tras PROMOTE_CTX=NO señaliza`.

PTOP del propio chip: `GR0 … runlist=0xd00000` y `CE0 … runlist=0xd00000`.
`CE1` está en `0xd00400`. El primer trabajo del canal de GR que no termina
deja esa runlist parada, y el CE de arranque es COPY0 (motor 9, clase
`0xcab5`), el de esa misma runlist. El golden y el `chan_init` de GR0 no
pararon el CE; lo para el clear.

La sonda «tras PROMOTE_CTX» (`gsp_bringup.c:1076-1093`) corre después del
selftest 3D, no justo después del promote. El nombre apunta al promote; el
corte medido es el clear.

El valor que suelta el semáforo 3D es `GSP_3D_REPORT_RELEASE_WRITES` (`0x10`,
`gsp_3d.h`). En `clc597.h:3806-3818` el bit 4 es `RELEASE` (eso es el `0x10`)
y `PIPELINE_LOCATION` son los bits 15:12. Con `0x10` quedan a cero
(`PIPELINE_LOCATION_NONE`). `ALL` es `0xF` en ese campo (`0xF000` en el
registro). Un release con etapa NONE no tiene de dónde retirarse: el semáforo
se queda en 0 y el canal no avanza. El CE, en la misma runlist, deja de
señalizar justo después. Encaja con el log. No hay un llamador de ese método
en el nouveau de referencia; la comparación es el layout del registro en
OGKM 570.144.

Descartado como causa: `RAMFC USERD/GPFIFO … NO_COINCIDE` y `INTR_0=badf5040`.
El volcado lee USERD en `+0x008` y el GPFIFO en `+0x048`
(`gsp_chan.c`, comentario «ga100/gv100»), que es donde escribe
`ga100_chan_ramfc_write` (`fifo/ga100.c:77`). En el COPY0 que ya había movido
11 veces el GPFIFO enviado (`0x8020000000`) está en `+0x090`, y `+0x008` es 0.
El `0xbadf5040` es el anillo PRI diciendo que `0xd00000` no se lee desde el
host; PTOP y la tabla del FIFO coinciden en esa base. El doorbell del log
(`kick=0x40000002`, bit 30 y chid) es el de `gb202_chan_doorbell_handle`
(`fifo/gb202.c`: `BIT(30) | (runl << 16) | chid`) con runlist 0. El CE funcionó
con ese kick antes del clear.

### H3 — la ethernet de la placa es RTL8125 y el driver no la reclama (confirmado, no es el corte)

`SOSODRV`: `83:00.0 10ec:8125 sin-driver`. Linux 6.6.32 la tiene en
`r8169_main.c:164` (`PCI_VDEVICE(REALTEK, 0x8125)`) con firmware RTL8125A/B
(`r8169_main.c:140-142`). soso solo lista `0x8168, 0x8161, 0x8162, 0x8167,
0x8136` (`kernel/src/drivers/rtl8169.rs:17`). `rtl8169::init` va después de
`boot: gpu` (`main.rs:241`), y este arranque no llegó. `net: sin NIC` sale
antes, durante el bring-up, cuando todavía no se ha probado Realtek.

## 4. Orden de corrección

1. **Reloj de `iwl_poll_bit` en microsegundos, como Linux.**
   `lxdde/ports/iwlwifi/iwl_trans.c` (`iwl_poll_bit`, `iwl_finish_nic_init`).
   25000 debe ser 25 ms, no 25 s. Una línea al entrar en `start_firmware`,
   antes de la espera. Host: el sondeo de 25000 del MAC no dura segundos.
   Placa: justo después del probe, `context-info gen3` o `MAC no despertó`,
   y el log sigue hasta `boot: task`.
2. **Clear 3D que no clave el CE.** `gsp_3d.h` / `gsp_3d.c`:
   `PIPELINE_LOCATION_ALL` (`0xF` en bits 15:12, `clc597.h:3818`) en el
   `SET_REPORT_SEMAPHORE_D`. Y el CE de arranque en CE1 (`runlist 0xd00400`,
   `ce_rebind_copy2` en `gsp_bringup.c`) antes del selftest, para que un
   canal GR parado no sea la runlist de las copias. Host: el pushbuffer del
   clear lleva `0xF000` junto al bit de release. Placa: `3D clear verificado`
   o, si el clear falla, la sonda CE de después sigue en `mueve`.
3. **Reclamar `10ec:8125` con el camino RTL8125 de Linux**, no añadiendo el
   ID al arranque del 8168H. `kernel/src/drivers/rtl8169.rs` y
   `r8169_main.c` (id en la línea 164, VER_61/63). Host: el PCI `10ec:8125`
   casa con el driver. Placa: `drv: 83:00.0 10ec:8125` compilado y enlace,
   en un arranque que ya haya pasado de `boot: gpu`.

## 5. Qué no se ha hecho

Ni el reloj del WiFi, ni el semáforo 3D, ni el 8125 están tocados. No hay
ciclo de placa de la corrección. Los hostchecks de firmware salieron en verde
y no sustituyen ese ciclo.
