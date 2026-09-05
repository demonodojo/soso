---
name: soso-live
description: >-
  soso live USB, native install and OTA updates — GPT image, ESP 8.3 slots,
  boot-shim, USB/xHCI mass storage, soso-install and soso-update. Use when
  modifying package-usb-live, flash-usb-live, boot-shim, updslot, bootreq,
  espfat, usb_storage, xhci-nostd, gptdisk, soso-update-core, or ESP files
  SOSOLOG/SOSODRV/SOSOBOOT/SOSOUPD/SOSOKRN/SOSOWIFI.
---

# soso — Live USB, instalación y actualizaciones

El camino de placa: **un GPT** (`soso-live.img`), no tres imágenes sueltas.
Docs de usuario: `MANUAL-USUARIO.md`. Bring-up on-box: `docs/L5c-on-box.md`.

## Particiones (orden real en el stick)

| # | Nombre | Contenido |
|---|--------|-----------|
| 1 | ESP (FAT) | `BOOTX64.EFI` (shim) + `bootsoso.efi` + huecos 8.3 |
| 2 | sosofs | rootfs |
| 4 | `SOSOINSTALL` (FAT) | `install-soso.sh` para Linux; **tras el rootfs, no al final** |
| 3 | sosomfs | modelos (se estira al flashear; grow del superbloque al montar) |

Linux monta p4, no la ESP. El log **no** está en p4: usa `cargo xtask sosolog`.

Al final del pendrive Linux no montaba p4 (LBA que leía ceros). Por eso
`SOSOINSTALL` va entre rootfs y modelos.

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
| `SOSOKRN.BIN` | 64 MiB | `soso-update` / shim | hueco kernel (nuevo o backup) |
| `kernel-x86_64` | — | shim al aplicar | kernel UEFI activo |

Pass 1 de `package-usb-live` reserva huecos; pass 2 **reutiliza** la misma
entrada FAT (no duplicar `SOSOUPD`/`SOSOKRN`).

## Boot-shim (`boot-shim/`)

`BOOTX64.EFI` deja marca en `BOOTMARK.TXT`, atiende buzones y chainloadea
`efi/boot/bootsoso.efi`.

- **Install:** `bootentry.rs` lee `SOSOBOOT.TXT`, crea `Boot####` «soso».
  El kernel no puede tocar NVRAM (`ExitBootServices`). Si falla, **no aborta**
  el arranque. Deja `DONE Boot#### soso`.
- **Update:** `actualiza.rs` lee `SOSOUPD.TXT`:
  - `KERNEL <tam> <sha256> <ver>` → verifica hueco, copia kernel viejo a
    `SOSOKRN.BIN`, escribe el nuevo, deja `PROBANDO`.
  - `PROBANDO` en el *siguiente* arranque = el anterior falló → revertir.
  - `OK` / `REVERTIR` / idle: ver `crates/soso-update-core/src/mailbox.rs`.
- Init confirma el kernel nuevo (`OK`) si el userspace llega.

Diagnóstico: `BOOTMARK` vacío → firmware; `BOOTMARK` escrito y `SOSOLOG`
vacío → kernel (checkpoints `boot:` en pantalla).

## Instalación nativa (`soso-install`)

Userspace: `user/coreutils/src/bin/soso-install.rs`.

1. `list` — `SYS_DISK_LIST`; clasifica particiones (`raw_disk`).
2. Destino **solo NVMe**, nunca el disco de arranque; `--force` si hay otro SO.
3. Clona el live; `gptdisk::relayout` (respaldo GPT al final, p3 estirada,
   **GUID nuevos** — si no, el firmware no distingue USB y destino).
4. `SYS_BOOTREQ_WRITE` → `INSTALL <guid>` en `SOSOBOOT.TXT`.
5. Reiniciar **con el USB**: el shim registra `Boot####`. Quitar USB.

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
- Kernel: `SYS_UPD_WRITE`/`READ` (68/69) sobre huecos ESP.
- Comandos: `estado` / `comprobar` / `aplicar` / `revertir`; `--local` apunta
  a un directorio (p. ej. `/var/actualiza-prueba`).
- Versión: `VERSION` → `/etc/soso-release` + banner kernel.

E2E: `cargo xtask test-update`.

## USB / xHCI (`crates/xhci-nostd` + `drivers/usb_storage.rs`)

Live en placa: GPT por **USB BOT** (`live: GPT backend=Usb`) o NVMe.

- DMA del event ring en **uncacheable** (`dma::alloc_zeroed_uc`). Write-back
  dejaba `EINT=1` y el software veía el anillo vacío (timeout + dump de puertos).
- **Longitud de Normal TRB = 17 bits:** `0x20000` (128 KiB) se desborda a 0 →
  Stall (`CSW inválido sig=0`). Tope `MAX_XFER = 64 KiB` en mass_storage.
  `raw_disk` usa 128 KiB y **debe** trocear antes del TRB.
- Bounce DMA **persistente** (`XhciController::bounce`): el asignador DMA del
  kernel no libera; uno por comando tiraba tanta RAM como datos movidos.
- `HOSTS` es `spin::Mutex` **no reentrante**. IRQ 1 / `poll_keyboard` no puede
  `lock()` (teclado muerto en placa; QEMU SSH no lo ve). Ver architecture.
- Se conservan todos los xHCI (un HCD por controlador, como Linux).

Tests: `cargo xtask test-usb` (4 escenarios en paralelo).

## Comandos host

```bash
cargo xtask package-usb-live              # qwen3.8-27b demo (sin medir stick)
cargo xtask flash-usb-live /dev/sdX --yes # mide, elige GGUF, dd, estira p3
SOSO_LIVE_OFFLINE=1 cargo xtask flash-usb-live /dev/sdX --yes
SOSO_QEMU_LIVE=1 cargo xtask run
cargo xtask sosolog [--drv] [/dev/sdX]
cargo xtask test-install
cargo xtask test-update
cargo xtask test-usb
```

No `sudo cargo`: root no tiene rustup. `sudo env "PATH=$PATH" "HOME=$HOME" …`

Perfil default live: `live-usb` (virtio + e1000e + rtl8169 + nvme + usb +
live-disk + nouveau + iwlwifi). Override: `SOSO_DRIVERS`.
