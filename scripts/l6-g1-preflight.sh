#!/usr/bin/env bash
# Diagnóstico G1 antes de VFIO: DMAR, GRUB, GPU, firmware rootfs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "=== L6 G1 preflight ==="
echo ""

echo "1) Placa"
if [[ -r /sys/class/dmi/id/product_name ]]; then
  echo "   $(cat /sys/class/dmi/id/sys_vendor 2>/dev/null) $(cat /sys/class/dmi/id/product_name)"
fi

echo ""
echo "2) GPU NVIDIA"
lspci -nn | rg 'NVIDIA|10de:' || echo "   FAIL: sin NVIDIA"

echo ""
echo "3) VT-d / DMAR (ACPI)"
if [[ -r /sys/firmware/acpi/tables/DMAR ]]; then
  echo "   GO    tabla DMAR presente (firmware expone VT-d)"
else
  echo "   BLOCK sin tabla DMAR"
  echo "         → Activa VT-d / Intel Virtualization Technology en BIOS"
  echo "         → MSI Vector 16 HX: BIOS → Advanced → Integrated Peripherals → VT-d → Enabled"
  echo "         → Guardar y reiniciar; luego: sudo ./scripts/l6-g1-enable-iommu.sh && sudo reboot"
fi

echo ""
echo "4) Parámetros IOMMU en kernel"
if rg -q 'intel_iommu=on|amd_iommu=on' /proc/cmdline 2>/dev/null; then
  echo "   GO    $(rg -o 'intel_iommu=[^ ]+|amd_iommu=[^ ]+|iommu=[^ ]+' /proc/cmdline)"
else
  echo "   BLOCK intel_iommu=on no está en /proc/cmdline"
  echo "         → sudo ./scripts/l6-g1-enable-iommu.sh && sudo reboot"
fi

groups=$(find /sys/kernel/iommu_groups -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
echo "   Grupos IOMMU activos: ${groups}"

echo ""
echo "5) GRUB (/etc/default/grub)"
if [[ -r /etc/default/grub ]]; then
  rg '^GRUB_CMDLINE' /etc/default/grub || true
fi

echo ""
echo "6) Checklist automático"
cd "$ROOT"
cargo xtask g1-check 2>/dev/null | grep -E '^(===|   GO|   BLOCK|=== Resumen)' || cargo xtask g1-check

echo ""
echo "=== Siguiente paso ==="
if [[ ! -r /sys/firmware/acpi/tables/DMAR ]]; then
  echo "1. BIOS: VT-d Enabled → reinicio"
  echo "2. sudo ./scripts/l6-g1-enable-iommu.sh → reinicio"
  echo "3. TTY (Ctrl+Alt+F3): sudo ./scripts/l6-g1-vfio-test.sh"
elif [[ "$groups" -eq 0 ]]; then
  echo "1. sudo ./scripts/l6-g1-enable-iommu.sh → reinicio"
  echo "2. TTY: sudo ./scripts/l6-g1-vfio-test.sh"
else
  echo "IOMMU listo. Desde TTY: sudo ./scripts/l6-g1-vfio-test.sh"
  echo "(Validación BAR0 sin IOMMU, solo desarrollo: sudo ./scripts/l6-g1-vfio-noiommu.sh)"
fi
