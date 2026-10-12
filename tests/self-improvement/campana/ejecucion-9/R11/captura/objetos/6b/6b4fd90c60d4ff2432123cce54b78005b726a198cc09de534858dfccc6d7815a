#!/usr/bin/env bash
# Libera la dGPU NVIDIA del driver propietario para poder bindearla a vfio-pci,
# sin reiniciar.
#
# IMPORTANTE — por qué NO se usa el unbind por sysfs:
#   `echo 0000:01:00.0 > /sys/bus/pci/drivers/nvidia/unbind` con nvidia_drm vivo
#   (fb0 = nvidia-drmdrmfb, Xorg con la GPU abierta) revienta el kernel:
#   drm_WARN_ON(!list_empty(&fb->filp_head)) ×3 y luego GPF en
#   drm_framebuffer_cleanup+0x8f (puntero envenenado dead000000000122).
#   La máquina se queda colgada y hay que resetear. Visto el 2026-07-24 20:22
#   con kernel 7.0.0-28-generic + nvidia 595.84 (RTX 5070, GB205).
#
# Descargar los módulos es seguro: si algo usa la GPU, `modprobe -r` devuelve
# EBUSY y no toca nada.
set -euo pipefail

STOP_DM=0
[[ "${1:-}" == "--stop-dm" ]] && STOP_DM=1

if [[ $(id -u) -ne 0 ]]; then
  echo "Ejecutar: sudo $0 [--stop-dm]" >&2
  exit 1
fi

BDF="${SOSO_G1_BDF:-01:00.0}"
FULL="0000:${BDF}"

echo "=== L6 G1 — liberar dGPU ${FULL} del driver propietario ==="

# 1) ¿Pinta la dGPU el panel? Si PRIME está en modo nvidia, descargar el driver
#    deja la pantalla muerta: abortamos.
if command -v prime-select >/dev/null 2>&1; then
  mode=$(prime-select query 2>/dev/null || echo desconocido)
  echo "PRIME: ${mode}"
  if [[ "$mode" == "nvidia" ]]; then
    echo "FAIL: PRIME en modo 'nvidia' — la dGPU pinta el panel y perderías pantalla." >&2
    echo "      sudo prime-select on-demand && sudo reboot   (o 'intel')" >&2
    exit 1
  fi
fi

# 2) ¿Quién tiene la GPU abierta? Informativo: el modprobe -r es el juez final.
if command -v nvidia-smi >/dev/null 2>&1; then
  apps=$(nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader 2>/dev/null || true)
  [[ -n "$apps" ]] && { echo "Procesos de cómputo en la GPU:"; echo "$apps" | sed 's/^/  /'; }
fi
users=$(lsof -t /dev/nvidia* 2>/dev/null | sort -u | tr '\n' ' ' || true)
[[ -n "${users// /}" ]] && echo "PIDs con /dev/nvidia* abierto: ${users}"

# 3) Parar el gestor de sesión gráfica (Xorg mantiene la dGPU abierta incluso en
#    PRIME on-demand). Solo desde un TTY: hacerlo desde un terminal del escritorio
#    mataría este mismo terminal.
if [[ "$STOP_DM" -eq 1 ]]; then
  tty_dev=$(tty 2>/dev/null || echo desconocido)
  if [[ "$tty_dev" != /dev/tty[0-9]* ]]; then
    echo "FAIL: --stop-dm requiere un TTY real (Ctrl+Alt+F3); estás en '${tty_dev}'." >&2
    echo "      Pararía el escritorio y con él este terminal." >&2
    exit 1
  fi
  echo "Parando display-manager (cierra la sesión gráfica)..."
  systemctl stop display-manager 2>/dev/null || true
  # Dar tiempo a que Xorg suelte los DRM handles antes de descargar módulos.
  for _ in $(seq 20); do
    pgrep -x Xorg >/dev/null 2>&1 || break
    sleep 0.5
  done
fi

# 4) Descargar la pila. Orden: consumidores primero, nvidia al final.
for m in nvidia_uvm nvidia_drm nvidia_modeset nvidia nouveau; do
  grep -q "^${m} " /proc/modules || continue
  echo "modprobe -r ${m}..."
  if ! modprobe -r "$m" 2>/tmp/l6-modprobe-err; then
    echo "FAIL: no se pudo descargar ${m}: $(cat /tmp/l6-modprobe-err)" >&2
    echo "      Refcount: $(awk -v m="$m" '$1==m{print $3" (usado por "$4")"}' /proc/modules)" >&2
    echo "      Cierra lo que use la GPU, o repite con --stop-dm desde un TTY." >&2
    exit 1
  fi
done

# 5) Verificar que la función VGA quedó sin driver.
if [[ -L "/sys/bus/pci/devices/${FULL}/driver" ]]; then
  cur=$(basename "$(readlink "/sys/bus/pci/devices/${FULL}/driver")")
  echo "FAIL: ${BDF} sigue en el driver '${cur}'." >&2
  exit 1
fi

echo "OK: ${BDF} sin driver — lista para vfio-pci"
echo "Siguiente: sudo ./scripts/l6-g1-vfio-test.sh"
echo "Volver al host: sudo ./scripts/l6-g1-vfio-restore.sh"
