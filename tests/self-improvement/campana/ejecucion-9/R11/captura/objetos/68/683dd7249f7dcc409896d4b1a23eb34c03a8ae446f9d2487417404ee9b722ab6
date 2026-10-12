#!/usr/bin/env bash
# Deshace l6-g1-vfio-test.sh: devuelve las funciones del slot de la dGPU a sus
# drivers nativos (nvidia + snd_hda_intel). Útil para volver al uso normal del
# host sin reiniciar.
set -euo pipefail

BDF="${SOSO_G1_BDF:-01:00.0}"
slot="${BDF%.*}"   # 01:00.0 → 01:00

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0   (o SOSO_G1_BDF=01:00.0 sudo $0)" >&2
  exit 1
fi

echo "=== L6 G1 — restaurar drivers nativos del slot ${slot} ==="

for path in /sys/bus/pci/devices/0000:${slot}.*; do
  [[ -e "$path" ]] || continue
  fn=$(basename "$path")        # 0000:01:00.x
  fbdf="${fn#0000:}"
  cur=""
  [[ -L "${path}/driver" ]] && cur=$(basename "$(readlink "${path}/driver")")

  if [[ "$cur" != "vfio-pci" ]]; then
    echo "${fbdf}: driver actual '${cur:-ninguno}' — nada que deshacer"
    continue
  fi

  # Quitar el device-id de vfio-pci para que no lo reclame en el rebind.
  ids=$(lspci -n -s "$fbdf" | awk '{print $3}')
  echo "${ids%%:*} ${ids##*:}" >/sys/bus/pci/drivers/vfio-pci/remove_id 2>/dev/null || true

  echo "${fbdf}: unbind vfio-pci..."
  echo "$fn" >/sys/bus/pci/drivers/vfio-pci/unbind 2>/dev/null || true

  # driver_override puede haber quedado fijado a vfio-pci; limpiarlo antes del probe.
  echo "" >"${path}/driver_override" 2>/dev/null || true

  echo "$fn" >/sys/bus/pci/drivers_probe 2>/dev/null || true

  new=""
  [[ -L "${path}/driver" ]] && new=$(basename "$(readlink "${path}/driver")")
  echo "${fbdf}: driver → ${new:-ninguno}"
done

# Si se liberó la GPU con l6-g1-nvidia-release.sh, los módulos no están cargados y
# drivers_probe no encuentra a quién dársela: recargamos la pila propietaria.
if ! grep -q '^nvidia ' /proc/modules; then
  echo ""
  echo "nvidia no está cargado — recargando la pila propietaria..."
  modprobe nvidia_drm 2>/dev/null || modprobe nvidia 2>/dev/null || \
    echo "WARN: no se pudo cargar nvidia; reinicia para restaurar el host" >&2
  for path in /sys/bus/pci/devices/0000:${slot}.*; do
    [[ -e "$path" ]] || continue
    [[ -L "${path}/driver" ]] || echo "$(basename "$path")" >/sys/bus/pci/drivers_probe 2>/dev/null || true
  done
fi

if ! systemctl is-active --quiet display-manager 2>/dev/null; then
  echo ""
  echo "display-manager parado — arrancarlo: sudo systemctl start display-manager"
fi

echo ""
echo "Estado final:"
lspci -nnk -s "$slot"
