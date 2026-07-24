#!/usr/bin/env bash
# Post-reinicio: bind VFIO de la dGPU NVIDIA y prueba NV_PMC_BOOT_0 en soso.
# Ejecutar desde TTY (Ctrl+Alt+F3), no desde sesión gráfica — pierdes la pantalla.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BDF="${SOSO_G1_BDF:-01:00.0}"
FULL="0000:${BDF}"

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0   (o SOSO_G1_BDF=01:00.0 sudo $0)" >&2
  exit 1
fi

echo "=== L6 G1 — prueba VFIO (${FULL}) ==="

groups=$(find /sys/kernel/iommu_groups -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
if [[ "$groups" -eq 0 ]]; then
  echo "FAIL: 0 grupos IOMMU. ¿Reiniciaste tras l6-g1-enable-iommu.sh y VT-d en BIOS?" >&2
  if [[ ! -r /sys/firmware/acpi/tables/DMAR ]]; then
    echo "      Sin tabla DMAR — activa VT-d en BIOS (MSI: Advanced → Integrated Peripherals → VT-d)" >&2
  fi
  echo "      Preflight: ./scripts/l6-g1-preflight.sh" >&2
  echo "      Fallback BAR0 (no oficial): sudo ./scripts/l6-g1-vfio-noiommu.sh" >&2
  exit 1
fi
echo "OK: ${groups} grupos IOMMU"

modprobe vfio-pci

ids=$(lspci -n -s "$BDF" | awk '{print $3}')
ven="${ids%%:*}"
dev="${ids##*:}"
echo "PCI ${BDF}: ${ven}:${dev}"

drv_link="/sys/bus/pci/devices/${FULL}/driver"
if [[ -L "$drv_link" ]]; then
  cur=$(basename "$(readlink "$drv_link")")
  if [[ "$cur" != "vfio-pci" ]]; then
    echo "Unbind ${cur}..."
    echo "$FULL" >"/sys/bus/pci/drivers/${cur}/unbind" 2>/dev/null || {
      echo "FAIL: no se pudo unbind ${cur}. Cierra sesión gráfica / usa TTY." >&2
      exit 1
    }
  fi
fi

if ! grep -q "${ven} ${dev}" /sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null; then
  echo "${ven} ${dev}" >/sys/bus/pci/drivers/vfio-pci/new_id
fi
echo "$FULL" >/sys/bus/pci/drivers/vfio-pci/bind

echo "OK: ${FULL} → vfio-pci"

# Permisos para el usuario que invocó sudo
if [[ -n "${SUDO_USER:-}" ]]; then
  uid=$(id -u "$SUDO_USER")
  gid=$(id -g "$SUDO_USER")
  chown "${uid}:${gid}" /dev/vfio/vfio 2>/dev/null || true
  for n in /dev/vfio/*; do
    [[ -e "$n" ]] && chown "${uid}:${gid}" "$n" 2>/dev/null || true
  done
fi

log="${ROOT}/target/g1-vfio-serial.log"
mkdir -p "${ROOT}/target"
rm -f "$log"

echo "Lanzando soso (timeout 90s, log → ${log})..."
cd "$ROOT"
run_user="${SUDO_USER:-$USER}"
sudo -u "$run_user" -- env SOSO_QEMU_GPU="vfio:${BDF}" \
  timeout 90 cargo xtask run >"$log" 2>&1 || true

if grep -q 'NV_PMC_BOOT_0=' "$log"; then
  grep 'nvidia:' "$log" || true
  echo ""
  echo "GO: NV_PMC_BOOT_0 legible desde soso."
  exit 0
fi

echo "FAIL o incompleto — revisar ${log}"
grep -E 'nvidia:|gpu:|VFIO|error|fail' "$log" | tail -20 || tail -20 "$log"
exit 1
