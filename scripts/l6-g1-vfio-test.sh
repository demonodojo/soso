#!/usr/bin/env bash
# Post-reinicio: bind VFIO de la dGPU NVIDIA y prueba NV_PMC_BOOT_0 en soso.
# En portátiles híbridos (iGPU Intel i915 pinta el panel) NO pierdes pantalla al
# pasar la dGPU. Aun así, hazlo sin apps usando la NVIDIA (nvidia-smi, CUDA, PRIME
# render offload); si el unbind de nvidia falla, usa un TTY (Ctrl+Alt+F3).
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

# VFIO exige que TODAS las funciones del dispositivo (y su grupo IOMMU) estén en
# vfio-pci o el grupo no es "viable". En una dGPU eso incluye la función de audio
# HDMI (p.ej. GPU 01:00.0 + audio 01:00.1). Bindeamos todas las funciones del slot.
slot="${BDF%.*}"   # 01:00.0 → 01:00
bound=0
for path in /sys/bus/pci/devices/0000:${slot}.*; do
  [[ -e "$path" ]] || continue
  fn=$(basename "$path")        # 0000:01:00.x
  fbdf="${fn#0000:}"            # 01:00.x
  fids=$(lspci -n -s "$fbdf" | awk '{print $3}')
  fven="${fids%%:*}"
  fdev="${fids##*:}"
  echo "Función ${fbdf}: ${fven}:${fdev}"
  drv_link="${path}/driver"
  if [[ -L "$drv_link" ]]; then
    cur=$(basename "$(readlink "$drv_link")")
    if [[ "$cur" != "vfio-pci" ]]; then
      echo "  unbind ${cur}..."
      echo "$fn" >"/sys/bus/pci/drivers/${cur}/unbind" 2>/dev/null || {
        echo "FAIL: no se pudo unbind ${cur} de ${fbdf}. Cierra apps que usen la GPU (nvidia-smi, CUDA) o usa un TTY." >&2
        exit 1
      }
    fi
  fi
  if ! grep -q "${fven} ${fdev}" /sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null; then
    echo "${fven} ${fdev}" >/sys/bus/pci/drivers/vfio-pci/new_id 2>/dev/null || true
  fi
  echo "$fn" >/sys/bus/pci/drivers/vfio-pci/bind 2>/dev/null || true
  bound=$((bound + 1))
done
echo "OK: ${bound} función(es) del slot ${slot} → vfio-pci"

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
