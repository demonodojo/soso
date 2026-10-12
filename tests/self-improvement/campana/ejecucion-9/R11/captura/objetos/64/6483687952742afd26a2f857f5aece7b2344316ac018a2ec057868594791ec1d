#!/usr/bin/env bash
# Passthrough VFIO de la WiFi Intel AX211 (CNVi) a QEMU para probar soso/iwlwifi.
#
# Uso:
#   sudo ./scripts/l6-wifi-vfio-test.sh
#   SOSO_WIFI_BDF=80:14.3 SOSO_WIFI_TIMEOUT=120 sudo ./scripts/l6-wifi-vfio-test.sh
#
# Requisitos: IOMMU activo (VT-d), módulo vfio-pci, iwlwifi descargado del host.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BDF="${SOSO_WIFI_BDF:-80:14.3}"
FULL="0000:${BDF}"
TIMEOUT="${SOSO_WIFI_TIMEOUT:-120}"
SERIAL_LOG="${SOSO_WIFI_SERIAL_LOG:-$ROOT/target/wifi-vfio-serial.log}"

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0" >&2
  exit 1
fi

echo "=== WiFi VFIO — Intel AX211 (${FULL}) ==="

groups=$(find /sys/kernel/iommu_groups -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
if [[ "$groups" -eq 0 ]]; then
  echo "FAIL: 0 grupos IOMMU. Activa VT-d en BIOS." >&2
  exit 1
fi

modprobe vfio-pci

path="/sys/bus/pci/devices/${FULL}"
if [[ ! -e "$path" ]]; then
  echo "FAIL: dispositivo ${FULL} no encontrado" >&2
  exit 1
fi

ids=$(lspci -n -s "$BDF" | awk '{print $3}')
ven="${ids%%:*}"
dev="${ids##*:}"
echo "Dispositivo: ${ven}:${dev}"

if [[ -L "${path}/driver" ]]; then
  cur=$(basename "$(readlink "${path}/driver")")
  if [[ "$cur" != "vfio-pci" ]]; then
    echo "Unbind ${cur}..."
    echo "$FULL" >"/sys/bus/pci/drivers/${cur}/unbind"
  fi
fi

if ! grep -q "${ven} ${dev}" /sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null; then
  echo "${ven} ${dev}" >/sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null || true
fi
echo "$FULL" >/sys/bus/pci/drivers/vfio-pci/bind
echo "OK: ${FULL} → vfio-pci"

cd "$ROOT"
cargo xtask lx-build iwlwifi

export SOSO_LXDDE=1
export SOSO_LXDDE_MODE=iwlwifi
export SOSO_QEMU_NIC="vfio:${BDF}"

echo "Arrancando QEMU (${TIMEOUT}s), log: ${SERIAL_LOG}"
rm -f "$SERIAL_LOG"
timeout "$TIMEOUT" env SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi \
  cargo xtask run -- --serial-log "$SERIAL_LOG" 2>&1 | tee "$SERIAL_LOG" || true

if grep -q "firmware ALIVE (UCODE_ALIVE_NTFY)" "$SERIAL_LOG" 2>/dev/null; then
  echo "GO: ALIVE real detectado"
  exit 0
fi
if grep -q "ALIVE degradado" "$SERIAL_LOG" 2>/dev/null; then
  echo "FAIL: ALIVE degradado ya no es válido" >&2
  exit 1
fi

echo "FAIL: revisa ${SERIAL_LOG}" >&2
exit 1
