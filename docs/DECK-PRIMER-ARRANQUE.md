# Steam Deck OLED — primer arranque

Guion del **único** viaje a la máquina que hace falta para desbloquear el resto
de la fase D. La Deck no tiene puerto serie: todo lo que no se traiga de este
arranque cuesta otro ciclo completo de flasheo. Por eso la lista está pensada
para volver con las cuatro cosas de golpe.

Plan completo: [`PLAN-STEAMDECK.md`](PLAN-STEAMDECK.md).

## 1. Preparar el pendrive

```sh
./scripts/l6-pack-ath11k-fw.sh                 # firmware WCN6855 hw2.1
SOSO_DRIVERS=deck cargo xtask package-usb-live
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes
```

El perfil `deck` deja fuera el firmware de NVIDIA y de Intel WiFi (66 MiB que
en esta máquina no sirven) y compila el port `ath11k`.

## 2. Arrancar

La Deck arranca de USB con **volumen abajo + encendido**, y elige el pendrive en
el menú. Secure Boot viene desactivado de fábrica, así que el shim UEFI entra
sin firmar nada.

Necesitas un **hub USB-C**: uno para el pendrive y, si quieres teclado, otro
puerto. Sin teclado externo se puede leer la pantalla igual — el objetivo de
este arranque es el volcado, no usar la máquina.

## 3. Qué mirar en la pantalla

Los marcadores `boot:` acotan dónde muere un arranque. En orden:

```
soso 0.2.2 (…)
fb: 800x1280 …            ← si dice 1280x800, el firmware ya rota: anota cuál
fb: rot=270 consola 1280x800
serial: COM1 0x3F8 ausente (consola = framebuffer + PS/2)
boot: memtest → acpi/smp → tsc → iommu → pci → nvme → usb → live-disk → kbd
boot: fs → ethernet → red → task
```

**Haz una foto de la pantalla.** Es el único dato que dice si la rotación es la
correcta: si el texto sale del revés (cabeza abajo), la rotación buena es `90`
y se arregla recompilando con `SOSO_FB_ROT=90`, sin tocar código.

Dos líneas concretas que hay que leer:

- `iommu: …` — `sin-ivrs` (no hay AMD-Vi visible), `apagado-por-firmware`
  (estaba apagado) o `desactivado-por-soso` (venía encendido y lo apagamos).
  Si dice **`SIGUE-ACTIVO`**, cualquier DMA posterior es sospechoso y ese es el
  primer problema a resolver.
- `usb: N xHCI activos ms=… kbd=…` — con el pendrive dentro, `ms` debe ser
  `true`. Si es `false` no hay live y el resto del arranque no ocurre.

## 4. Qué traerse

Con el pendrive de vuelta en el PC:

```sh
cargo xtask sosolog                      # SOSOLOG.TXT + SOSODRV.TXT de la ESP
# o a mano:
udisksctl mount -b /dev/sdX1 && cp /media/*/SOSO{LOG,DRV}.TXT /tmp/deck/
```

Los cuatro datos que desbloquean el trabajo siguiente:

| Dato | Dónde | Para qué |
|---|---|---|
| Volcado PCI completo | líneas `drv:` de `SOSODRV.TXT` | Confirmar los IDs reales (APU, WiFi `17cb:1103`, xHCI, NVMe, lector SD) |
| Estado del IOMMU | línea `iommu:` de `SOSOLOG.TXT` | Saber si el DMA que programamos va a llegar |
| Descriptores USB | líneas `usb: dev/if/rep` de `SOSODRV.TXT` | El report descriptor del mando (`28de:1205`): decide si el teclado emulado se puede leer con el camino boot o hace falta parsear el descriptor |
| Foto de la pantalla | tu móvil | Sentido de la rotación |

Registrar el resultado en la matriz:

```sh
cargo xtask hw-matrix parse-logs --id deck-oled \
    --sosolog /tmp/deck/SOSOLOG.TXT --sosodrv /tmp/deck/SOSODRV.TXT
cargo xtask hw-matrix record-boot --id deck-oled     # o --fail
```

## 5. Lo que **no** hay que esperar de este arranque

- **Red no va a haber.** ath11k está en W1 (sólo transporte MHI) y no hay
  Ethernet en la máquina. Sin red no hay SSH ni arnés: es esperado, no un fallo.
- La línea `lxdde: ath11k start rc=… fase=…` dirá hasta dónde llegó el
  transporte. Cualquier fase distinta de `mission_mode` es información útil,
  no un fracaso: `sin_firmware`, `bhi_off`, `execenv`, `ready`, `ctxt`,
  `fw_download` y `espera_amss` señalan exactamente dónde se rompió.
- La GPU no se toca. La consola es el framebuffer del firmware.

## 6. Si no arranca nada

Si la pantalla se queda negra o el logo de Valve no da paso a soso, el fallo
está antes de que exista consola. Por orden de probabilidad:

1. El pendrive no entró en el menú de arranque (repite con volumen abajo).
2. El shim UEFI no encontró el kernel: reflashea con `--only kernel`.
3. El kernel murió antes de `fb::init`. Sin serie no hay traza; el siguiente
   paso sería un arranque con la ESP de un pendrive ya conocido para descartar
   el flasheo.
