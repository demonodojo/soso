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
echo "2b) Audio HDMI NVIDIA (VFIO / NVDisplay N8)"
bdf="${SOSO_G1_BDF:-01:00.0}"
slot="${bdf%.*}"
audio="0000:${slot}.1"
gpu="0000:${slot}.0"
if [[ -e "/sys/bus/pci/devices/${audio}" ]]; then
  aid=$(lspci -n -s "${slot}.1" 2>/dev/null | awk '{print $3}' || true)
  echo "   Función audio: ${slot}.1 (${aid:-?})"
  if [[ "$aid" == "10de:2f80" ]] || [[ "$aid" == 10de:* ]]; then
    gg=$(readlink -f "/sys/bus/pci/devices/${gpu}/iommu_group" 2>/dev/null || true)
    ga=$(readlink -f "/sys/bus/pci/devices/${audio}/iommu_group" 2>/dev/null || true)
    if [[ -n "$gg" && "$gg" == "$ga" ]]; then
      echo "   GO    audio HDMI en el mismo grupo IOMMU que la dGPU (bind VFIO en l6-g1-vfio-test)"
    else
      echo "   AVISO grupos IOMMU distintos GPU vs audio — revisar antes de VFIO"
    fi
  fi
else
  echo "   (sin función .1 en slot ${slot}; Ampere de sobremesa suele omitir audio HDMI)"
fi
echo "   Audio HDMI no usa NVDisplay en soso (F6 aparte); documentado en docs/L6-G1-gate.md"

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
groups=$(find /sys/kernel/iommu_groups -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
if rg -q 'intel_iommu=on|amd_iommu=on' /proc/cmdline 2>/dev/null; then
  echo "   GO    $(rg -o 'intel_iommu=[^ ]+|amd_iommu=[^ ]+|iommu=[^ ]+' /proc/cmdline)"
elif [[ "${groups}" -gt 0 ]]; then
  # Kernels recientes activan intel_iommu solos si hay DMAR: los grupos son la prueba.
  echo "   GO    IOMMU activo por defecto del kernel (sin intel_iommu=on en cmdline)"
else
  echo "   BLOCK intel_iommu=on no está en /proc/cmdline"
  echo "         → sudo ./scripts/l6-g1-enable-iommu.sh && sudo reboot"
fi

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
