# Plan Steam Deck OLED — fase D

Fecha: **9 de septiembre de 2026**. Versión en árbol: **0.2.2**.
Objetivo del ciclo: **soso arrancando en una Steam Deck OLED (Galileo)** con
consola legible, entrada, disco y SSH por WiFi nativo. La GPU RDNA2 queda
**fuera** de este plan.

Estado al 9 de septiembre de 2026: **D0, D1, D2 y W1 cerrados** en todo lo que
no exige la máquina; el detalle está al final de cada sección. Lo que queda es
el primer arranque en la Deck (§9) y, con lo que traiga, W2–W5.

Referencia obligatoria en todo el documento: el árbol Linux 6.6.32 en
[`lxdde/linux/`](../lxdde/linux). Cada decisión cita el fichero de Linux del que
sale. Nada se «deduce» del comportamiento observado sin contrastarlo ahí.

## 1. Inventario del objetivo

Steam Deck OLED, nombre de placa **Galileo**, APU **Sephiroth** (6 nm, Zen 2
4c/8t + RDNA2 8 CU), 16 GB LPDDR5.

| Pieza | Identificación | Driver Linux | Estado en soso |
|---|---|---|---|
| APU / iGPU | `1002:163f` (Van Gogh; rev. distinta en Galileo — confirmar en hwscan) | `amdgpu` | Fuera de alcance; se usa el GOP del firmware |
| Panel interno | 800×1280 **nativo vertical**, montado girado | quirk `lcd800x1280_rightside_up` en [`drm_panel_orientation_quirks.c:417`](../lxdde/linux/drivers/gpu/drm/drm_panel_orientation_quirks.c) | **Hecho**: rotación automática en `fb.rs` (D1) |
| WiFi | **QCNFA765 = WCN6855 hw2.1**, `17cb:1103` | `ath11k_pci`, firmware `ath11k/WCN6855/hw2.1/` ([`ath11k/pci.c:29`](../lxdde/linux/drivers/net/wireless/ath/ath11k/pci.c), [`core.c:462`](../lxdde/linux/drivers/net/wireless/ath/ath11k/core.c)) | **W1 hecho** (transporte MHI); W2–W5 pendientes |
| Mandos integrados | USB `28de:1205` en hub interno | `hid-steam` ([`hid-ids.h:1225`](../lxdde/linux/drivers/hid/hid-ids.h)) | **Hecho** lo que no depende de la placa (D2) |
| SSD | NVMe M.2 2230, PCIe | `nvme` | `drv-nvme` existe; ya soporta LBA de 512 y 4096 |
| microSD | Controlador SD del FCH (confirmar BDF en hwscan) | `sdhci-pci` | Falta (opcional) |
| USB | xHCI del FCH AMD + hub interno | `xhci_hcd` | `xhci-nostd` existe |
| Audio | ACP + I2S (`cs35l41`), **no** HD-Audio | `snd_sof_amd` | Fuera de alcance |
| Serie | **no hay** | — | Consola = pantalla; log = `SOSOLOG` en la ESP |

Dos consecuencias que ordenan todo lo demás:

- **No hay puerto serie.** Igual que en el ROG, cada iteración en placa cuesta
  un ciclo completo de flasheo y lectura de la ESP. Por eso todo lo que pueda
  probarse en QEMU o en host debe probarse antes de tocar la Deck.
- **No hay Ethernet.** Hasta que ath11k funcione no hay SSH, ni arnés, ni
  `cargo xtask test` contra la placa. El bring-up de D0–D3 va a ciegas
  contra `SOSOLOG`/`SOSODRV`.

## 2. Lo que ya sirve sin tocar nada

Auditado leyendo el árbol, no supuesto:

- **MADT con x2APIC**: `parse_madt` ya acepta el tipo 9 (Processor Local x2APIC)
  en [`arch/acpi.rs:171`](../kernel/src/arch/acpi.rs). Zen 2 con 8 hilos no lo
  necesitará, pero si el firmware lo publica así, no rompe.
- **LAPIC en modo x2APIC**: [`arch/apic.rs`](../kernel/src/arch/apic.rs) ya
  escribe por MSR `0x800 + off/16`, no por MMIO. Un firmware que deje `EXTD`
  puesto no provoca #GP.
- **Sin i8042**: `kbd::init` consulta `i8042_present()` y sale limpio
  ([`drivers/kbd.rs:502`](../kernel/src/drivers/kbd.rs)); los bucles de
  `mouse.rs` están acotados por iteraciones, no esperan indefinidamente.
- **Sin HD-Audio**: `hda::init` recorre PCI buscando clase 04:03 y no encuentra
  nada; no hay camino que se cuelgue.
- **Sin PIT que tickee**: `main.rs` cae al timer LAPIC si `pit::ticks() == 0`.
- **Varios xHCI**: `usb_storage::init` ya itera todos los controladores
  ([`drivers/usb_storage.rs:143`](../kernel/src/drivers/usb_storage.rs)).
- **Sin COM1**: `serial::present()` ya lo contempla y la consola cae al GOP.

Es decir: el arranque hasta `boot: pci` **debería** sobrevivir en la Deck sin
cambios. El plan empieza donde eso deja de ser cierto.

## 3. D0 — Que el arranque no muera en silencio

**Riesgo abierto: IOMMU AMD-Vi activo por firmware.** El kernel de soso no mira
la IVRS. Si el firmware de la Deck deja el IOMMU habilitado con tablas de
dispositivo propias, todo DMA que programemos (NVMe, xHCI, y luego el WiFi) se
aborta sin más síntoma que un dispositivo mudo — indistinguible de un driver
mal escrito, y el peor modo de fallo posible para depurar a ciegas.

Referencia Linux: [`drivers/iommu/amd/init.c`](../lxdde/linux/drivers/iommu/amd/init.c),
`iommu_disable()` y el registro de control MMIO (`MMIO_CONTROL_OFFSET` 0x18,
bit `CONTROL_IOMMU_EN`), con la tabla IVRS localizada por `acpi_table_parse`.

**Trabajo.**

1. `kernel/src/arch/iommu.rs`: localizar IVRS por el mismo camino que MADT/MCFG,
   enumerar los IVHD, leer el registro de control de cada IOMMU por su BAR y
   **reportar el estado** en el arranque y en `SOSODRV`.
2. Si está habilitado y no lo programamos nosotros, desactivarlo (limpiar
   `IommuEn`, `EventLogEn`, `CmdBufEn`) antes de `pci::init`. Es lo que hace
   Linux al tomar el control del hardware antes de reprogramarlo.
3. Registro de IDs conocidos de la Deck en `drivers/registry.rs` para que el
   hwscan diga «reconocido / sin driver» en vez de callarse: WiFi `17cb:1103`,
   iGPU `1002:163f`, xHCI y SD del FCH AMD.
4. Entrada `deck-oled` en `docs/hw-matrix.json`.

**Aceptación.** En QEMU, `SOSODRV` clasifica los IDs de la Deck como conocidos
sin driver; el camino IVRS se ejerce con una tabla sintética en test de host.
En placa: `SOSOLOG` llega a `boot: task` y lista el inventario PCI completo.

**Estado: hecho.** [`kernel/src/arch/iommu.rs`](../kernel/src/arch/iommu.rs) con
el parseo en [`crates/soso-hw/src/ivrs.rs`](../crates/soso-hw/src/ivrs.rs)
(8 tests de host: tipos 10h/11h duplicados, bloque de longitud cero, bloque que
desborda la tabla, y que el apagado limpia exactamente los bits de
`iommu_disable()`). Verificado además **contra un AMD-Vi emulado**
(`SOSO_QEMU_IOMMU=1 cargo xtask fb-shot`), que publica una IVRS real: el kernel
la parsea, mapea el MMIO y lee el registro de control —
`iommu: ivhd0x10 devid 0018 mmio 0xfed80000 control 0x0`. El único camino que
QEMU no puede provocar es el de un IOMMU que llegue encendido; ése queda
cubierto por el test de host y sólo la placa lo confirmará.

## 4. D1 — Pantalla legible: rotación del framebuffer

El panel es **800×1280 vertical nativo** montado girado; el GOP entrega ese
buffer tal cual, así que la consola sale de lado y el bring-up a ciegas se queda
sin su único canal. Linux lo resuelve con el quirk DMI citado arriba.

**Trabajo.** Rotación 0/90/180/270 en [`drivers/fb.rs`](../kernel/src/drivers/fb.rs):

1. El shadow buffer ya existe y es la vista lógica; la rotación se aplica sólo
   al volcar al GOP. Las celdas de texto se siguen calculando sobre el ancho y
   alto **lógicos** (intercambiados cuando la rotación es 90/270).
2. Desactivar el camino rápido de scroll: con rotación 90 el desplazamiento deja
   de ser un `memcpy` de líneas contiguas. Repintado completo desde el shadow.
3. Respetar `copy_to_gop` (nada de `movaps` sobre el GOP de AMD; ya documentado
   en `fb.rs:144`).
4. Política: autodetección `height > width → 270`, con anulación explícita desde
   el arranque para no depender de adivinar el sentido correcto.

**Aceptación.** Test de host de la transformación de coordenadas en las cuatro
rotaciones (incluida la inversa, para el cursor). QEMU forzando un GOP vertical:
banner legible y scroll correcto. El sentido real (270 frente a 90) sólo lo
confirma la placa: son dos líneas de configuración, no dos implementaciones.

**Estado: hecho.** El shadow buffer pasó a ser la vista lógica y el volcado
aplica la rotación recorriendo líneas físicas
([`kernel/src/drivers/fb.rs`](../kernel/src/drivers/fb.rs), geometría en
[`crates/soso-hw/src/fbrot.rs`](../crates/soso-hw/src/fbrot.rs) con 10 tests:
biyección de las cuatro rotaciones, inversa exacta y recorte). Verificado
**mirando la pantalla**: `cargo xtask fb-shot` captura el framebuffer del guest
por el monitor de QEMU y lo deja en PNG; con `SOSO_FB_ROT=270` la consola sale
girada y, al deshacer la rotación, el log se lee entero y en orden. `auto`
(la Deck) sale de la forma del panel; `SOSO_FB_ROT` la anula sin tocar código.

## 5. D2 — Entrada sin teclado: los mandos en «lizard mode»

Hallazgo que ahorra un driver entero: el mando de la Deck **arranca emulando
teclado y ratón** por HID. Es el «lizard mode» documentado en
[`hid-steam.c:15`](../lxdde/linux/drivers/hid/hid-steam.c), activo por defecto
(`static bool lizard_mode = true`, línea 51); el driver de Linux lo **desactiva**
al abrir el dispositivo. Si no escribimos ningún driver Valve, el hardware sigue
comportándose como teclado/ratón USB — que es exactamente lo que `xhci-nostd`
ya sabe consumir.

Con un matiz que hay que comprobar en placa: que esa emulación exponga el
**protocolo boot** y no sólo el modo report. `hid.rs` implementa hoy sólo boot.

**Trabajo.**

1. Enumerar más allá del puerto raíz: el mando cuelga de un **hub interno**.
   `mass_storage.rs` ya recorre hubs para el stick; extender esa ruta al HID.
2. Aceptar HID en interfaces distintas de la 0 (el mando expone varias).
3. **Volcar a `SOSODRV` los descriptores** de configuración e interfaz de todo lo
   que aparezca en el bus. Ese volcado es el que decide el punto 4, y sale gratis
   en el primer arranque en placa.
4. Si no hay protocolo boot: parseo mínimo de report descriptor, acotado a
   localizar los usages de teclado. No un stack HID completo.

**Aceptación.** QEMU con `usb-kbd` detrás de un hub: teclas en `sosh`. El
descriptor real del `28de:1205` es **metal obligatorio** — es el primer dato que
hay que traerse del primer arranque.

**Estado: hecho lo que no depende de la placa.** La enumeración ya recorría
hubs y aceptaba HID genérico en cualquier interfaz (`find_hid_keyboard` admite
protocolo report, no sólo boot). Añadido: (a) `SET_PROTOCOL(Boot)` **sólo** en
interfaces con la subclase Boot, como hace `usbhid_start` — un HID que sólo
habla report protocol puede contestar STALL, y el mando de la Deck es
justamente ese caso; (b) lectura del **report descriptor** de cada interfaz HID
(`GET_DESCRIPTOR(0x22)`) y volcado del inventario USB completo — dispositivo,
interfaces y descriptor en hexadecimal — a `SOSODRV.TXT`, junto al hwscan.
El punto 4 (parseo del report descriptor) queda deliberadamente **sin escribir**
hasta ver el descriptor real: escribirlo antes sería programar contra un
descriptor imaginario.

Verificado en QEMU con un teclado detrás de un hub (`SOSO_QEMU_USB_KBD=1
SOSO_QEMU_USB_HUB=1 cargo xtask fb-shot`): el informe lista el hub, el teclado
que cuelga de él y sus 63 bytes de report descriptor en hexadecimal, y el
camino boot sigue intacto (`boot=true`, `SET_PROTOCOL(Boot) succeeded`).

## 6. D3 — Disco

El OLED lleva siempre NVMe (no hay variante eMMC como en la LCD de 64 GB), y
`drv-nvme` ya funciona. Queda verificar lo que cambia entre un NVMe de QEMU y
uno real: tamaño de bloque declarado por el namespace (512 frente a 4096) y que
`live_disk` localice la ESP del USB con el NVMe interno presente y poblado con
las particiones de SteamOS.

microSD (`sdhci-pci`) queda como **opcional**: no hace falta para arrancar, y
QEMU lo emula, así que se puede añadir después sin coste de placa.

**Aceptación.** `cargo xtask test-usb` y `test-install` en QEMU con un segundo
disco NVMe poblado. En placa: arrancar del USB sin tocar el SSD de SteamOS.

## 7. D4 — WiFi nativo: ath11k / WCN6855 hw2.1

Es el 80 % del ciclo. Y es honesto decirlo antes de empezar: **ath11k es
sustancialmente más grande que iwlwifi**. No comparte casi nada con lo ya
portado — ni el transporte, ni el protocolo de control, ni el camino de datos.

| Capa | Fichero Linux | Qué obliga a implementar |
|---|---|---|
| Transporte | [`mhi.c`](../lxdde/linux/drivers/net/wireless/ath/ath11k/mhi.c), `pci.c` | Bus MHI: BHI/BHIe, descarga de `amss.bin`, anillos de canal, canales `LOOPBACK` e `IPCR` |
| Control | [`qmi.c`](../lxdde/linux/drivers/net/wireless/ath/ath11k/qmi.c) (86 KB) | QMI sobre QRTR: capacidades de host y target, descarga de `board-2.bin` y `m3.bin`, modo WLAN |
| Copia | `ce.c`, `pcic.c` | Copy Engines y su reparto de interrupciones |
| Mensajería | `htc.c` | Control por créditos |
| Firmware | [`wmi.c`](../lxdde/linux/drivers/net/wireless/ath/ath11k/wmi.c) (281 KB) | `service_ready`, `ready`, creación de vdev/peer, scan |
| Datos | `dp.c`, `dp_rx.c` (158 KB), `hal.c` | Anillos SRNG del HAL y descriptores de rx/tx |

**Subfases**, cada una con su propio criterio de parada:

- **W1 — MHI.** PCI, BAR, MSI-X, secuencia BHI y arranque del firmware hasta
  `MHI_STATE_MISSION_MODE`. Cierra cuando el target reporta estado válido.
- **W2 — QMI.** Handshake completo hasta `WLFW_WLAN_MODE`. Es donde el
  `board_id` decide qué recorte de `board-2.bin` enviar.
- **W3 — HTC + WMI.** Hasta el evento `ready` con la MAC del target.
- **W4 — HAL/DP.** Anillos, y un scan que devuelva beacons reales.
- **W5 — Asociación.** WPA2 reutilizando [`net/wifi_wpa.rs`](../kernel/src/net/wifi_wpa.rs)
  (el EAPOL de iwlwifi ya está escrito), DHCP y SSH.

**Estructura.** `lxdde/ports/ath11k/` en C estilo Linux, igual que el port
iwlwifi: no se compila el driver de Linux tal cual, se reimplementa acotado
contra él. Firmware empaquetado en `rootfs/lib/firmware/ath11k/WCN6855/hw2.1/`
por el mismo camino que GSP e iwlwifi.

**Cómo se prueba sin Deck.** `tools/ath11k-hostcheck/`, siguiendo el patrón de
`iwl-hostcheck` y `gsp-hostcheck`, pero un paso más allá: en vez de stubs
mudos, compila el port contra un **modelo del dispositivo** que imita la máquina
de estados del arranque (PBL → BHIe → AMSS → M0). Los accesos MMIO son la
costura (`ath11k_mmio.c` en el kernel, el modelo en el hostcheck). **Cubre la
lógica, no el silicio**: W1 y W2 no se cierran sin placa.

**Estado de W1: escrito y verde contra el modelo.**
[`lxdde/ports/ath11k/`](../lxdde/ports/ath11k) implementa el transporte —
reset, espera de READY, contextos de canal/evento/comando, publicación de
CCABAP/ECABAP/CRCBAP, descarga de firmware por BHIe con tabla de vectores y
paso a M0. `scripts/l6-ath11k-hostcheck.sh`: **38 comprobaciones, 0 fallos**,
incluidos el arranque completo, el rearranque con el chip ya en AMSS, la
recuperación de SYS_ERR (el caso QCA6390 tras reset global, con INTVEC
reprogramado), y cuatro modos de fallo que deben **rendirse en vez de colgar el
arranque**: bus mudo (0xffffffff), BHIOFF fuera del BAR, firmware rechazado y
un dispositivo que nunca llega a READY. El layout de los contextos se comprueba
byte a byte (44 bytes, `rbase/rlen/rp/wp` en 12/20/28/36): si el compilador
alineara los u64 a 8, el dispositivo leería relleno.

Falta para cerrar W1 en placa: empaquetar `amss.bin` de `WCN6855/hw2.1` en
`rootfs/lib/firmware` y comprobar que los offsets y los tiempos reales son
éstos.

## 8. D5 — Integración y evidencia

Entrada `deck-oled` en la matriz con las etapas WiFi de siempre (ALIVE → scan →
WPA2 → DHCP → SSH → reconexión), hashes de firmware en el árbol, `record-boot` y
`record-bench` con el LLM en CPU. Manual de usuario al cerrar D2 (la Deck se
maneja con el mando, no con teclado: eso es documentación de usuario).

## 8 bis. Dos cosas que aparecieron al probar

**El perfil de la Deck necesitó desacoplar la capa lxdde de nouveau.** El
módulo GPU de `kernel/src/lxdde` referenciaba los símbolos `lx_nouveau_*` sin
condición, así que cualquier perfil que compilara lxdde **sin** el port nouveau
no enlazaba — y el de la Deck usa lxdde sólo para el WiFi. Ahora los módulos
`gpu` y `wifi` de esa capa se eligen por feature (`lxdde-nouveau`,
`lxdde-iwlwifi`) y, cuando su port no está, se compila un doble con la misma
superficie que responde «no hay ese hardware» en vez de fingir que lo hay.
`xtask` activa cada feature según los ports del perfil.

**El arranque BIOS no carga un kernel con lxdde.** Con el perfil `deck`, la
imagen `soso-bios.img` se queda muda: el bootloader no llega ni a escribir en
la serie, mientras que la misma imagen UEFI arranca hasta `boot: task`. No
afecta a la Deck —que arranca por UEFI, como el resto de placas— pero conviene
saberlo antes de perder una tarde: `cargo xtask fb-shot` usa ahora la imagen
UEFI siempre que haya OVMF. Queda sin diagnosticar por estar fuera del camino.

## 9. Orden de ejecución y el momento «metal»

```
D0 ─┬─ D1 ── D2 ─── (primer arranque en placa: descriptores HID + IVRS + PCI)
    └─ D3
                 D4 (W1…W5) ── D5
```

D0–D3 son días y se validan casi enteros en QEMU y en host. Se cierran **antes**
de encender la Deck, porque el primer arranque en placa debe traerse de una vez:
el volcado PCI completo, el estado del IOMMU, los descriptores USB del mando y
una foto de la pantalla que diga qué rotación es la correcta. Con eso, D4 arranca
sabiendo contra qué.

**El plan es explícito en esto:** ath11k no se puede cerrar sin placa, pero W1 y
W2 tampoco se pueden empezar bien sin ese primer volcado. Por eso el orden no es
negociable — y por eso el trabajo se detiene, por diseño, en cuanto D0–D3 estén
verdes y sólo quede encender la máquina.
