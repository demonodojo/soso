# L5c — Bring-up en placa (on-box)

Arranque **live desde USB** sin modificar el Linux del disco interno, o **dual-boot**
con un segundo disco dedicado.

## Resumen

| Componente | Estado |
|------------|--------|
| Imagen live GPT (`soso-live.img`) | `cargo xtask package-usb-live` |
| USB live + instalador (`install-soso.sh`) | `sudo cargo xtask flash-usb-live /dev/sdX --yes` |
| Instalación desde soso live | `soso-install list` / `soso-install nvme1 --yes` |
| GRUB tras install live | `install-soso.sh --grub-only /dev/nvmeXn1` desde Linux |
| Montaje part2/3 vía GPT (USB / NVMe) | `live_disk` en kernel |
| Validación QEMU | `SOSO_QEMU_LIVE=1 cargo xtask run` |
| USB BOT (mismo stick en placa) | Implementado — `live: GPT backend=Usb` |
| NVMe dedicado (dual-boot) | Implementado — `live: GPT backend=Nvme(0)` |
| NVMe interno con Linux | **No tocar** — usar otro disco o USB live |

## Dual-boot (segundo disco + GRUB)

Si tienes un NVMe/SSD **vacío** aparte del Linux:

```bash
lsblk   # identificar el disco, p. ej. /dev/nvme1n1 — NO /dev/nvme0n1

# Con cargo en la máquina de desarrollo:
sudo cargo xtask install-disk /dev/nvme1n1 --yes

# O con USB live que incluye instalador (sin cargo en el PC destino):
sudo cargo xtask flash-usb-live /dev/sdX --yes   # flashear pendrive
# … Linux en marcha, USB conectado …
sudo /media/$USER/SOSO_INSTALL/install-soso.sh /dev/nvme1n1 --yes

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
3. **sosomfs** — modelos (tamaño según `SOSO_MODELS_DIR` / `SOSO_MODELS_SIZE`)

El disco NVMe/SSD con Linux **no se toca**.

### Host — generar imagen

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

### Limitación actual (placa real)

Tras arrancar desde USB, el kernel debe leer las particiones 2/3 **del mismo stick** vía **USB mass storage**. Hoy:

- xHCI se detecta e inicializa (RUN).
- **BOT/read aún no implementado** → en placa real puede fallar el montaje live.

**Workaround temporal:** segundo disco NVMe vacío dedicado a soso (modo clásico `package-usb`, o dual-boot con `install-disk`).

**Validación sin placa:** QEMU simula el disco live con virtio:

```bash
SOSO_QEMU_LIVE=1 cargo xtask run
# Log esperado: live: GPT … | fs: sosofs live | sosh —
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
| 7 | Inferencia | `soso-llm run …` |
| 8 | Reboot sin USB | Linux host intacto |

## Variables útiles

| Variable | Efecto |
|----------|--------|
| `SOSO_MODELS_DIR` | Árbol `.som` al empaquetar |
| `SOSO_MODELS_SIZE` | Tamaño imagen modelos (default 8G) |
| `SOSO_QEMU_LIVE=1` | QEMU con `soso-live.img` |
| `SOSO_FIRMWARE=uefi` | Arranque OVMF en QEMU |
| `SOSO_QEMU_NIC=e1000e` | NIC física en QEMU |

## Siguiente hito

Implementar **USB BOT read** en [`kernel/src/drivers/usb_storage.rs`](../kernel/src/drivers/usb_storage.rs) para cerrar live en placa sin segundo disco.
