#!/usr/bin/env bash
# G1 fallback SIN IOMMU: solo validar NV_PMC_BOOT_0 (modo inseguro vfio noiommu).
# Usar solo si VT-d no está disponible aún. El cierre G1 oficial requiere IOMMU.
# Ejecutar desde TTY (Ctrl+Alt+F3).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BDF="${SOSO_G1_BDF:-01:00.0}"
FULL="0000:${BDF}"

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0" >&2
  exit 1
fi

echo "=== L6 G1 — VFIO no-IOMMU (solo BAR0, NO es cierre G1 oficial) ==="
echo "WARN: enable_unsafe_noiommu_mode — sin aislamiento DMA"
echo ""

modprobe vfio
echo 1 >/sys/module/vfio/parameters/enable_unsafe_noiommu_mode
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
      echo "FAIL: no se pudo unbind ${cur}. Usa TTY sin sesión gráfica." >&2
      exit 1
    }
  fi
fi

echo "${ven} ${dev}" >/sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null || true
echo "$FULL" >/sys/bus/pci/drivers/vfio-pci/bind
echo "OK: ${FULL} → vfio-pci (noiommu)"

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

run_user="${SUDO_USER:-$USER}"
echo "Lanzando soso como ${run_user} (timeout 90s)..."
cd "$ROOT"
sudo -u "$run_user" -- env SOSO_QEMU_GPU="vfio:${BDF}" \
  timeout 90 cargo xtask run >"$log" 2>&1 || true

if grep -q 'NV_PMC_BOOT_0=0x' "$log"; then
  grep 'nvidia:' "$log" || true
  echo ""
  echo "PARTIAL: NV_PMC_BOOT_0 legible (noiommu). Cierra G1 oficial con VT-d + IOMMU."
  exit 0
fi

echo "FAIL — revisar ${log}"
grep -E 'nvidia:|gpu:|VFIO|error|fail|qemu' "$log" | tail -25 || tail -25 "$log"
exit 1
