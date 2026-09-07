# Guía operativa (desarrollo y placa)

Recorrido coherente con `cargo xtask`. Estado y versiones: [`ESTADO.md`](ESTADO.md).

## 1. Compilar

Desde la raíz del repo, **sin `sudo cargo`** (root no tiene rustup):

```sh
cargo xtask build
cargo xtask check          # recomendado antes de commit o release
```

## 2. QEMU

```sh
cargo xtask run            # consola serie en stdio
# otra terminal:
ssh -tt -i target/soso_test_key -p 2222 soso@localhost
```

Salir de QEMU: `Ctrl-A X`. Si el puerto 2222 está ocupado: `pkill qemu-system-x86`.

Integración completa:

```sh
cargo xtask test
cargo xtask test-update    # OTA en OVMF (requiere ovmf)
```

## 3. Diagnóstico

| Síntoma | Acción |
|---------|--------|
| Fallo en arranque QEMU | Log del shard: `target/test-*-serial.log` |
| Live USB en placa | `cargo xtask sosolog [/dev/sdX]` |
| hwscan / drivers | `cargo xtask sosolog --drv [/dev/sdX]` |
| Host GPU/WiFi parser | `./scripts/l6-g3-gsp-hostcheck.sh`, `./scripts/l6-iwl-fw-hostcheck.sh` |
| Matriz de placa | `./scripts/l6-a8-collect.sh --id <nombre> --boot-ok` |

## 4. Live USB

Identifica el dispositivo (`lsblk`). Sustituye `/dev/sdX`:

```sh
cargo xtask flash-usb-live /dev/sdX --yes
```

Actualización incremental (kernel/rootfs, conserva modelos en p3):

```sh
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models
```

Offline / sin red al empaquetar:

```sh
sudo env SOSO_LIVE_OFFLINE=1 "PATH=$PATH" "HOME=$HOME" \
  cargo xtask flash-usb-live /dev/sdX --yes
```

WiFi antes del primer arranque: editar `SOSOWIFI.TXT` en la ESP (partición 1) o
`/etc/wifi.conf` en el rootfs antes de empaquetar.

## 5. Instalación en NVMe

Desde soso live: `soso-install` (ver manual de usuario).

Desde Linux anfitrión (dual-boot):

```sh
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask install-disk /dev/nvmeXn1 --yes
```

## 6. Validación en hardware (A8)

```sh
cargo xtask hw-matrix init    # primera vez
./scripts/l6-a8-collect.sh --id mi-placa --equipo "..." --pci 10de:2f18 --boot-ok
cargo xtask hw-matrix show
```

GPU VFIO (sesión preparada): ver [`board.txt`](../board.txt) → enlace a notas históricas.

## Referencias

- Usuario final: [`MANUAL-USUARIO.md`](../MANUAL-USUARIO.md)
- Desarrollo detallado: skill `soso-dev` (`.claude/skills/soso-dev/`)
- Live/OTA: skill `soso-live`
- Matriz hardware: [`HW-MATRIX.md`](HW-MATRIX.md)
