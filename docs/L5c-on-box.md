# L5c — Bring-up en placa (on-box)

Arranque **live desde USB** sin modificar el Linux del disco interno, o **dual-boot**
con un segundo disco dedicado.

## Resumen

| Componente | Estado |
|------------|--------|
| Imagen live GPT (`soso-live.img`) | `cargo xtask package-usb-live` |
| USB live + instalador (`install-soso.sh`) | `sudo cargo xtask flash-usb-live /dev/sdX --yes` |
| Instalación desde soso live | `soso-install list` / `soso-install <id> --yes` |
| Reparticionado del destino | `crates/gptdisk` — respaldo al final, p3 llena el disco, GUID nuevos |
| Entrada de arranque UEFI | La registra el shim en el arranque siguiente (`SOSOBOOT.TXT` → `Boot####`) |
| GRUB tras install live | Opcional: `install-soso.sh --grub-only /dev/nvmeXn1` desde Linux |
| Validación end-to-end del instalador | `cargo xtask test-install` (3 arranques OVMF) |
| Montaje part2/3 vía GPT (USB / NVMe) | `live_disk` en kernel |
| Validación QEMU | `SOSO_QEMU_LIVE=1 cargo xtask run` |
| USB BOT (mismo stick en placa) | Implementado — `live: GPT backend=Usb` |
| NVMe dedicado (dual-boot) | Implementado — `live: GPT backend=Nvme(0)` |
| NVMe interno con Linux | **No tocar** — usar otro disco o USB live |

## Dual-boot desde el propio soso (sin Linux)

El camino normal: se instala desde el live y la placa se queda con una entrada
de arranque propia. No se escribe un solo sector en el disco de Linux.

```bash
# arrancado desde el pendrive live:
soso-install list        # enseña las particiones de cada disco antes de borrar
soso-install 3 --yes     # id del NVMe destino

# reiniciar CON el USB puesto → el shim registra Boot#### «soso»
# apagar, quitar el USB → «soso» en el menú de arranque de la placa
```

Detalles del reparticionado y de la petición de arranque en
`MANUAL-USUARIO.md` → «Instalar soso en un disco».

## Dual-boot desde Linux (segundo disco + GRUB)

Alternativa si prefieres que sea GRUB quien elija. Con un NVMe/SSD **vacío**
aparte del Linux:

```bash
lsblk   # identificar el disco, p. ej. /dev/nvme1n1 — NO /dev/nvme0n1

# Con cargo en la máquina de desarrollo:
sudo cargo xtask install-disk /dev/nvme1n1 --yes

# O con USB live que incluye instalador (sin cargo en el PC destino):
sudo cargo xtask flash-usb-live /dev/sdX --yes   # flashear pendrive
# … Linux en marcha, USB conectado …
sudo /media/$USER/SOSOINSTALL/install-soso.sh /dev/nvme1n1 --yes

# reiniciar → menú GRUB → "soso"
```

Layout del disco (igual que USB live):

1. **ESP** — kernel UEFI
2. **sosofs** — rootfs
3. **sosomfs** — modelos

GRUB de Linux hace chainload a `/EFI/BOOT/BOOTX64.EFI` en la ESP de ese disco.
El disco con Linux **no se modifica** (salvo el snippet `/etc/grub.d/41_soso`).

Desinstalar la entrada GRUB:

```bash
sudo rm /etc/grub.d/41_soso && sudo update-grub
```

Ver también [`MANUAL-USUARIO.md`](../MANUAL-USUARIO.md) (sección dual-boot).

## Modo live (recomendado para pruebas sin tocar discos)

Un solo pendrive con tres particiones:

1. **ESP** — kernel UEFI (~22 MiB)
2. **sosofs** — rootfs (~64 MiB)
3. **sosomfs** — modelos (**TinyLlama 1.1B Chat** + `tiny` sintético, ~2 GiB por defecto)

Tras `cargo xtask flash-usb-live`, si el pendrive es más grande que la imagen,
**p3 se estira** hasta dejar 32 MiB al final para p4 `SOSOINSTALL`; al arrancar,
sosomfs hace **grow** del superbloque para usar ese espacio (import HF con
`soso-hf pull`).

El disco NVMe/SSD con Linux **no se toca**.

### Host — generar imagen

```bash
# Modelo real (solo la primera vez si falta target/tinyllama-model)
cargo xtask fetch-hf TinyLlama/TinyLlama-1.1B-Chat-v1.0

# Firmware Ampere (ga107 + ga102 fallback)
./scripts/l6-pack-firmware.sh

# Imagen live con perfil live-usb (nouveau + drv-gpu-nvidia + firmware Ampere y Blackwell)
cargo xtask package-usb-live
# o flashear directo:
sudo cargo xtask flash-usb-live /dev/sdX --yes

ls -lh target/usb-live/
#   soso-live.img
#   FLASH-LIVE.txt
```

`package-usb-live` y `flash-usb-live` usan el perfil **`live-usb`** por defecto
(lxdde + nouveau + firmware ga107/ga102 y gb205). Override: `SOSO_DRIVERS=…`.

En placa, `ask` o `soso-llm run tinyllama --prompt "hola" --max 32 --chat` demuestran
texto real. Override de modelos: `SOSO_MODELS_DIR=… SOSO_MODELS_SIZE=…`.

**El modelo empaquetado tiene que ser v5 o `ask` contestará como si continuara un
texto**: la plantilla de chat va en el manifiesto, y los `.som` convertidos antes
del 2026-08-17 no la llevan. Se arregla reconvirtiendo desde el GGUF cacheado,
sin volver a descargar:

```bash
cargo run --release -p convert-gguf -- target/tinyllama-q4km.gguf target/tinyllama-model --name tinyllama
```

### Prueba GPU en placa (mismo stick, Ampere o Blackwell)

Tras arrancar desde USB (sin VFIO, sin tocar el NVMe de Linux):

1. Consola / `SOSOLOG.TXT` en ESP part1: `nvidia: GPU 10de:…`.
2. Criterio G1: `NV_PMC_BOOT_0` distinto de `0xffffffff`.
   - ROG / RTX 3050 Mobile (`249c`): `nouveau-lx: familia=ga107` (fw ga107 o fallback ga102).
   - RTX 5070 Ti Mobile (`2f18`): `nouveau-lx: familia=Blackwell` (fw gb205).
3. SSH → `hwscan` (debe listar `gpu-nvidia` / `lx-nouveau`).

Verde mínimo: BAR0 vivo + firmware de esa familia. Amarillo aceptable en primera
tanda Ampere: ACR soft-fail o `GSP booted (soft)` si BAR0 ya responde.

**En Ampere el offload no va a correr todavía, y ahora se ve por qué.** La cadena
RM → VMM → canal/CE → pool de VRAM sólo existe en la rama Blackwell/FMC de
`gsp_bringup.c`: con un ga107 el bring-up llega a `GSP booted (hw poll ok)` y
vuelve, así que no hay pool y las reservas de VRAM fallan a propósito en vez de
repartir memoria del kernel disfrazada. Se reconoce por:

- Arranque: `gpu: NVIDIA detectada (chipset …, GSP=booted, pool VRAM=no)`.
- Inferencia: `soso-llm: GPU presente sin pool de VRAM (fase=booted) — inferencia en CPU`.

Si en vez de eso sale `offload GPU desactivado — subida de pesos`, es otra cosa:
el camino de subida falló con pool disponible (2026-08-17 era el techo de 16 MiB
de la syscall, ya quitado).

```bash
# Modelo tiny por defecto; modelos grandes:
# SOSO_MODELS_DIR=/ruta/al/modelo SOSO_MODELS_SIZE=32G cargo xtask package-usb-live

cargo xtask package-usb-live
ls -lh target/usb-live/
#   soso-live.img
#   FLASH-LIVE.txt
```

### Host — flashear USB

```bash
lsblk   # identificar el stick, p. ej. /dev/sde — NO /dev/nvme0n1
sudo dd if=target/usb-live/soso-live.img of=/dev/sdX bs=4M status=progress conv=fsync
```

### Placa — arranque

1. UEFI → Boot once desde USB (F12 / menú de arranque).
2. Consola serie o GOP: buscar líneas:
   - `live: GPT backend=… root LBA …`
   - `fs: sosofs live`
   - `fs: sosomfs live`
   - `net: dhcp …`
3. Desde el host: `ssh -i target/soso_test_key soso@<ip>`
4. Probar: `soso-llm run tiny --prompt hola --max 8`
5. Apagar, quitar USB, arrancar disco habitual → Linux intacto.

### Requisitos del pendrive (placa real)

Tras arrancar desde USB, el kernel lee las particiones 2/3 **del mismo stick** vía **USB mass storage (BOT)**. Requisitos:

- Interfaz **BOT** (`8/6/0x50`); **UAS** (`8/6/0x62`) no soportado.
- Bloques de **512 B**; nativos 4K se descartan.
- El pendrive puede estar en un **puerto root** o **detrás de un hub USB** (enumeración unificada HID + BOT).
- Tras HCRST el kernel espera a que vuelvan **todos** los puertos root que UEFI tenía conectados (no basta con que reaparezca solo el teclado interno).

Si falla el montaje, busca en serie/`SOSOLOG.TXT`:

- `usb: mass storage …` + `live: GPT backend=Usb` + `fs: sosofs live` → OK
- `usb: … teclado HID (sin mass storage)` + `live: sin GPT sosofs` → BOT no atado (hub, recovery, UAS, etc.)
- `live: GPT backend=Usb` + `fs: live sosofs falló` → imagen/partición 2 corrupta
- `xhci: bulk … Stall Error` con `CSW inválido sig=0` → transferencia mayor de la que
  admite un Normal TRB (longitud de 17 bits). El troceo a 64 KiB está en
  `mass_storage::MAX_XFER`; si vuelve a aparecer, es que algún camino lo saltó.
- `lxdde-fw: cargado …/ga102/…` cuando la GPU es un **ga107** → **no es un fallo,
  es el diseño**. Los cuatro blobs GSP de `ga107` y `ga102` en linux-firmware son
  **byte a byte idénticos** (mismo md5) y `ga107/acr/*` son symlinks a
  `ga102/acr/*`, así que el fallback carga exactamente los mismos bytes. Se
  decidió **no empaquetar `ga107`** (2026-08-16): duplicaría el
  `gsp-570.144.bin` de 63 MiB y sumaría ~64 MiB a la imagen live para nada.
  `lx_request_firmware` ya no imprime los intentos fallidos (llenaban el
  arranque de «no encontrado» que parecían averías); el fallo de verdad lo canta
  el llamante: `nouveau-lx: GSP … incompleto (n/m blobs)` o
  `iwl_ax211: firmware no encontrado`.

**Validación sin placa:** QEMU simula el disco live con virtio o USB:

```bash
SOSO_QEMU_LIVE=1 cargo xtask run
# Log esperado: live: GPT … | fs: sosofs live | sosh —

SOSO_QEMU_LIVE=1 SOSO_QEMU_LIVE_USB=1 cargo xtask run
# BOT en puerto root xHCI

cargo xtask test-usb
# Incluye escenario «BOT hub + teclado HID» (storage detrás de usb-hub)

cargo xtask test-install
# Instalación nativa completa: instalar por SSH, comprobar la GPT del destino
# desde el host, registrar Boot#### con el shim y arrancar solo del NVMe.
```

## Modo clásico (segundo disco, manual)

Si prefieres copiar las imágenes a mano en lugar de `install-disk`:

```bash
cargo xtask package-usb
# dd soso-uefi.img → USB
# dd soso-data.img → NVMe dedicado
# dd soso-models.img → segundo NVMe
```

Ver `target/usb-package/FLASH.txt`.

## Checklist bring-up

| # | Paso | Criterio |
|---|------|----------|
| 1 | `package-usb-live` | `soso-live.img` generado |
| 2 | `dd` → USB | Solo `/dev/sdX` del stick |
| 3 | Boot UEFI USB | Log kernel `soso 0.1` |
| 4 | ACPI/PCI | MCFG, ECAM, NIC PCI ID |
| 5 | Live mount | `fs: sosofs live` + modelos listados |
| 6 | Red | DHCP, ping/SSH |
| 7 | Inferencia | `ask hola` contesta **como asistente** (la plantilla de chat sale del modelo); `soso-llm run tinyllama --prompt hola --max 32 --chat` para lo mismo con diagnóstico |
| 7b | GPU (Ampere o Blackwell) | `10de:249c` → `familia=ga107`; `10de:2f18` → `familia=Blackwell`; `NV_PMC_BOOT_0 ≠ ffffffff` |
| 7c | Offload: dice la verdad | Línea de arranque `pool VRAM=sí/no`; con `no`, `ask` dice `GPU presente sin pool de VRAM (fase=…)` y **nunca** `subida de pesos` |
| 8 | Reboot sin USB | Linux host intacto |
| 9 | `soso-install list` | Lista particiones de cada disco; el de Linux sale como `OTRO` |
| 10 | `soso-install <id> --yes` | `copia terminada` + `GPT ajustada al disco` |
| 11 | Reboot con USB puesto | `soso-shim: DONE Boot#### soso` (y en `SOSOBOOT.TXT`) |
| 12 | Reboot sin USB | «soso» en el menú de la placa; `fs: sosofs live` desde el NVMe |

## Variables útiles

| Variable | Efecto |
|----------|--------|
| `SOSO_MODELS_DIR` | Árbol `.som` al empaquetar |
| `SOSO_MODELS_SIZE` | Tamaño imagen modelos (default 8G) |
| `SOSO_QEMU_LIVE=1` | QEMU con `soso-live.img` |
| `SOSO_QEMU_LIVE_USB=1` | Mismo disco vía xHCI + `usb-storage` (BOT) |
| `SOSO_QEMU_USB_HUB=1` | Storage/kbd detrás de `usb-hub` (prueba placa) |
| `SOSO_FIRMWARE=uefi` | Arranque OVMF en QEMU |
| `SOSO_QEMU_NIC=e1000e` | NIC física en QEMU |

## Siguiente hito

Ejecutar el checklist de arriba **en la placa real**, incluido el camino de
instalación nativa (`soso-install` → reinicio con el USB → entrada UEFI →
arranque sin pendrive). En QEMU con OVMF pasa entero (`cargo xtask
test-install`); lo que falta es confirmar que el firmware de la placa acepta el
`SetVariable` del shim, o si hay que caer al menú de arranque manual.
