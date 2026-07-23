#!/usr/bin/env bash
# Activa IOMMU en GRUB (Intel). Requiere root. Reinicia después.
set -euo pipefail

GRUB=/etc/default/grub
PARAMS='intel_iommu=on iommu=pt'

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0" >&2
  exit 1
fi

if ! grep -q GenuineIntel /proc/cpuinfo 2>/dev/null; then
  echo "CPU no Intel — edita manualmente: amd_iommu=on iommu=pt" >&2
  exit 1
fi

cp -a "$GRUB" "${GRUB}.bak.$(date +%Y%m%d%H%M%S)"

if grep -q 'intel_iommu=on' "$GRUB"; then
  echo "GRUB ya contiene intel_iommu=on"
else
  if grep -q '^GRUB_CMDLINE_LINUX=' "$GRUB"; then
    sed -i "s|^GRUB_CMDLINE_LINUX=.*|GRUB_CMDLINE_LINUX=\"${PARAMS}\"|" "$GRUB"
  else
    echo "GRUB_CMDLINE_LINUX=\"${PARAMS}\"" >>"$GRUB"
  fi
  echo "Añadido a GRUB_CMDLINE_LINUX: ${PARAMS}"
fi

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
echo "Listo. También activa VT-d / Intel Virtualization Technology en BIOS si no lo está."
echo "Reinicia: sudo reboot"
echo "Tras reinicio: cd $(dirname "$0")/.. && cargo xtask g1-check"
