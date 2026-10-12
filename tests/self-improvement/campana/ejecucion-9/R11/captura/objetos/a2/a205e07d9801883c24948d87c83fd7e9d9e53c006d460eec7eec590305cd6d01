# Notas históricas de placa (archivo)

Notas de sesiones locales; **no** son procedimiento oficial. Para el flujo
actual usa [`../GUIA-OPERATIVA.md`](../GUIA-OPERATIVA.md) y [`../HW-MATRIX.md`](../HW-MATRIX.md).

Sustituye `<DISPOSITIVO>` por tu pendrive/NVMe (`lsblk`).

---

## kdump (host Linux)

Tras reinicio con kdump habilitado:

```sh
./scripts/l6-kdump-setup.sh --status
sudo ./scripts/l6-kdump-setup.sh --selftest
```

Los sysctl se aplican en caliente con `--enable`; la reserva de memoria requiere reinicio.

## VFIO GPU (G1 ciclo)

Deshabilitar NVIDIA en el host:

```sh
sudo ./scripts/l6-g1-vfio-persist.sh --enable
sudo reboot
./scripts/l6-g1-vfio-persist.sh --status   # driver in use: vfio-pci
```

Ciclo de prueba (ajusta `SOSO_G1_BDF`):

```sh
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask build

sudo SOSO_G1_BDF=01:00.0 SOSO_G1_TIMEOUT=300 \
  SOSO_G1_CMD=$'init test\nsoso-llm run tiny --prompt test --max 4' \
  ./scripts/l6-g1-vfio-test.sh
```

Logs: `target/g1-vfio-serial.log`, `target/g1-vfio-serial-cmd.log`.

Devolver GPU al host:

```sh
sudo ./scripts/l6-g1-vfio-persist.sh --disable && sudo reboot
```

## Reflashear USB

```sh
sudo env "PATH=$PATH" "HOME=$HOME" \
  SOSO_MODELS_DIR=$PWD/target/tinyllama-model SOSO_MODELS_SIZE=2G \
  cargo xtask flash-usb-live <DISPOSITIVO> --yes

sudo env SOSO_LIVE_OFFLINE=1 "PATH=$PATH" "HOME=$HOME" \
  cargo xtask flash-usb-live <DISPOSITIVO> --yes

# Solo kernel:
sudo env "PATH=$PATH" "HOME=$HOME" \
  cargo xtask flash-usb-live <DISPOSITIVO> --yes --only kernel
```
