#!/usr/bin/env bash
# Activa IOMMU en GRUB (Intel). Requiere root. Reinicia después.
# Ubuntu: añade parámetros a GRUB_CMDLINE_LINUX_DEFAULT y GRUB_CMDLINE_LINUX.
set -euo pipefail

GRUB=/etc/default/grub
PARAMS='intel_iommu=on iommu=pt'

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0" >&2
  exit 1
fi

if ! grep -q GenuineIntel /proc/cpuinfo 2>/dev/null; then
  PARAMS='amd_iommu=on iommu=pt'
fi

if [[ ! -r /sys/firmware/acpi/tables/DMAR ]] && [[ "$PARAMS" == intel* ]]; then
  echo "WARN: no hay tabla DMAR en ACPI — VT-d probablemente desactivado en BIOS." >&2
  echo "      Habilita VT-d antes del reinicio o IOMMU seguirá en 0 grupos." >&2
  echo "      MSI Vector 16 HX: Advanced → Integrated Peripherals → VT-d → Enabled" >&2
  echo ""
fi

cp -a "$GRUB" "${GRUB}.bak.$(date +%Y%m%d%H%M%S)"

append_grub_param() {
  local key="$1"
  local line
  line=$(grep "^${key}=" "$GRUB" 2>/dev/null || true)
  if [[ -z "$line" ]]; then
    echo "${key}=\"${PARAMS}\"" >>"$GRUB"
    echo "Creado ${key}=\"${PARAMS}\""
    return
  fi
  if echo "$line" | grep -q 'intel_iommu=on\|amd_iommu=on'; then
    echo "${key} ya contiene IOMMU"
    return
  fi
  sed -i "s|^${key}=\"\\(.*\\)\"|${key}=\"\\1 ${PARAMS}\"|" "$GRUB"
  echo "Añadido a ${key}: ${PARAMS}"
}

append_grub_param GRUB_CMDLINE_LINUX_DEFAULT
append_grub_param GRUB_CMDLINE_LINUX

echo ""
echo "--- /etc/default/grub ---"
grep '^GRUB_CMDLINE' "$GRUB"

if command -v update-grub >/dev/null; then
  update-grub
elif command -v grub-mkconfig >/dev/null; then
  grub-mkconfig -o /boot/grub/grub.cfg
else
  echo "WARN: no update-grub — ejecuta grub-mkconfig manualmente" >&2
  exit 1
fi

echo ""
echo "Listo. También activa VT-d en BIOS si /sys/firmware/acpi/tables/DMAR no existe."
echo "Reinicia: sudo reboot"
echo "Tras reinicio: cd $(dirname "$0")/.. && ./scripts/l6-g1-preflight.sh"
