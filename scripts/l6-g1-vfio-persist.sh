#!/usr/bin/env bash
# Bind persistente de la dGPU NVIDIA a vfio-pci en el arranque, para iterar en G1
# sin cerrar el escritorio cada vez.
#
# Con esto nadie reclama la dGPU: el driver propietario queda en blacklist y
# vfio-pci la coge por vendor:device antes de que nvidia pueda cargarse. Así se
# evita por completo el unbind en caliente, que cuelga el kernel (GPF en
# drm_framebuffer_cleanup — ver l6-g1-nvidia-release.sh).
#
# Precio: en el host no hay CUDA ni nvidia-smi hasta hacer --disable. El panel
# sigue funcionando porque lo pinta la iGPU Intel (se verifica antes de tocar nada).
#
#   sudo ./scripts/l6-g1-vfio-persist.sh --enable    → configurar + reiniciar
#   sudo ./scripts/l6-g1-vfio-persist.sh --disable   → deshacer + reiniciar
#   ./scripts/l6-g1-vfio-persist.sh --status         → qué hay puesto ahora
set -euo pipefail

BDF="${SOSO_G1_BDF:-01:00.0}"
slot="${BDF%.*}"          # 01:00.0 → 01:00
FULL="0000:${BDF}"

MODPROBE_CONF=/etc/modprobe.d/soso-l6-vfio.conf
LOAD_CONF=/etc/modules-load.d/soso-l6-vfio.conf
GRUB=/etc/default/grub
BL_MODS="nvidia,nvidia_drm,nvidia_modeset,nvidia_uvm,nouveau"
CMD_PARAM="modprobe.blacklist=${BL_MODS}"

action="${1:---status}"

need_root() {
  if [[ $(id -u) -ne 0 ]]; then
    echo "Ejecutar: sudo $0 ${action}" >&2
    exit 1
  fi
}

# --- IDs del slot: todas las funciones (GPU + audio HDMI) ---------------------
slot_ids() {
  local ids="" path fbdf id
  for path in /sys/bus/pci/devices/0000:${slot}.*; do
    [[ -e "$path" ]] || continue
    fbdf=$(basename "$path"); fbdf="${fbdf#0000:}"
    id=$(lspci -n -s "$fbdf" | awk '{print $3}')
    [[ -n "$id" ]] && ids+="${ids:+,}${id}"
  done
  echo "$ids"
}

# --- Salvaguardas ------------------------------------------------------------
check_safe() {
  if [[ ! -e "/sys/bus/pci/devices/${FULL}" ]]; then
    echo "FAIL: no existe ${FULL}. Ajusta SOSO_G1_BDF." >&2
    exit 1
  fi

  # 1) La función objetivo debe ser NVIDIA: no vamos a poner en blacklist la iGPU.
  local vend
  vend=$(lspci -n -s "$BDF" | awk '{print $3}' | cut -d: -f1)
  if [[ "$vend" != "10de" ]]; then
    echo "FAIL: ${BDF} no es NVIDIA (vendor ${vend}) — abortado." >&2
    exit 1
  fi

  # 2) Debe haber OTRA GPU con driver de pantalla, o al reiniciar no hay panel.
  local other=""
  local card dev
  for card in /sys/class/drm/card[0-9]*; do
    [[ -e "$card/device" ]] || continue
    dev=$(basename "$(readlink -f "$card/device")")
    [[ "$dev" == "$FULL" ]] && continue
    [[ -L "/sys/bus/pci/devices/${dev}/driver" ]] || continue
    other="${dev} ($(basename "$(readlink "/sys/bus/pci/devices/${dev}/driver")"))"
    break
  done
  if [[ -z "$other" ]]; then
    echo "FAIL: no encuentro otra GPU con driver activo — sin la dGPU te quedas sin pantalla." >&2
    echo "      Aborto. Usa la vía temporal: l6-g1-nvidia-release.sh desde un TTY." >&2
    exit 1
  fi
  echo "GPU que pintará el panel: ${other}"

  # 3) PRIME en modo nvidia = el panel depende de la dGPU.
  if command -v prime-select >/dev/null 2>&1; then
    local mode
    mode=$(prime-select query 2>/dev/null || echo desconocido)
    if [[ "$mode" == "nvidia" ]]; then
      echo "FAIL: PRIME en modo 'nvidia'. Cambia antes: sudo prime-select on-demand" >&2
      exit 1
    fi
    echo "PRIME: ${mode}"
  fi
}

# --- GRUB: añadir/quitar modprobe.blacklist ----------------------------------
grub_backup() { cp -a "$GRUB" "${GRUB}.bak.$(date +%Y%m%d%H%M%S)"; }

grub_add() {
  local key line
  for key in GRUB_CMDLINE_LINUX_DEFAULT GRUB_CMDLINE_LINUX; do
    line=$(grep "^${key}=" "$GRUB" 2>/dev/null || true)
    [[ -z "$line" ]] && continue
    if echo "$line" | grep -q 'modprobe\.blacklist='; then
      # Reemplazar el valor existente por el nuestro (idempotente).
      sed -i "s|modprobe\.blacklist=[^ \"]*|${CMD_PARAM}|" "$GRUB"
      echo "${key}: modprobe.blacklist actualizado"
    else
      sed -i "s|^${key}=\"\\(.*\\)\"|${key}=\"\\1 ${CMD_PARAM}\"|" "$GRUB"
      echo "${key}: añadido ${CMD_PARAM}"
    fi
  done
}

grub_del() {
  local key
  for key in GRUB_CMDLINE_LINUX_DEFAULT GRUB_CMDLINE_LINUX; do
    grep -q "^${key}=" "$GRUB" 2>/dev/null || continue
    sed -i "s| *modprobe\.blacklist=${BL_MODS}||" "$GRUB"
  done
  echo "GRUB: modprobe.blacklist eliminado"
}

regen() {
  if command -v update-initramfs >/dev/null; then
    update-initramfs -u -k all
  fi
  if command -v update-grub >/dev/null; then
    update-grub
  elif command -v grub-mkconfig >/dev/null; then
    grub-mkconfig -o /boot/grub/grub.cfg
  else
    echo "WARN: sin update-grub — regenera grub.cfg a mano" >&2
  fi
}

# --- Acciones ----------------------------------------------------------------
do_enable() {
  need_root
  echo "=== L6 G1 — bind persistente de ${FULL} a vfio-pci ==="
  check_safe

  local ids
  ids=$(slot_ids)
  if [[ -z "$ids" ]]; then
    echo "FAIL: no pude leer los IDs del slot ${slot}" >&2
    exit 1
  fi
  echo "Funciones del slot ${slot} → vfio-pci: ${ids}"

  cat >"$MODPROBE_CONF" <<EOF
# soso L6 G1 — dGPU ${FULL} reservada para VFIO. Generado por
# scripts/l6-g1-vfio-persist.sh; deshacer con --disable.
options vfio-pci ids=${ids}
softdep nvidia pre: vfio-pci
blacklist nvidia
blacklist nvidia_drm
blacklist nvidia_modeset
blacklist nvidia_uvm
blacklist nouveau
EOF
  echo "Escrito ${MODPROBE_CONF}"

  # modules-load.d asegura que vfio-pci se cargue y reclame los ids en el arranque.
  printf 'vfio-pci\n' >"$LOAD_CONF"
  echo "Escrito ${LOAD_CONF}"

  # El blacklist de modprobe.d no frena una carga por nombre (initramfs, gpu-manager):
  # el parámetro de kernel sí la frena en todos los casos.
  grub_backup
  grub_add
  regen

  echo ""
  echo "Listo. Reinicia: sudo reboot"
  echo "Tras el reinicio, comprobar:"
  echo "  ./scripts/l6-g1-vfio-persist.sh --status"
  echo "  sudo ./scripts/l6-g1-vfio-test.sh      # ya la encontrará en vfio-pci"
  echo ""
  echo "OJO: en el host no habrá CUDA ni nvidia-smi hasta:"
  echo "  sudo $0 --disable && sudo reboot"
}

do_disable() {
  need_root
  echo "=== L6 G1 — devolver ${FULL} al host ==="

  rm -f "$MODPROBE_CONF" "$LOAD_CONF"
  echo "Borrados ${MODPROBE_CONF} y ${LOAD_CONF}"

  if grep -q 'modprobe\.blacklist=' "$GRUB" 2>/dev/null; then
    grub_backup
    grub_del
  else
    echo "GRUB: no había modprobe.blacklist"
  fi
  regen

  echo ""
  echo "--- /etc/default/grub ---"
  grep '^GRUB_CMDLINE' "$GRUB" || true
  echo ""
  echo "Reinicia para recuperar la dGPU en el host: sudo reboot"
  echo "(Sin reiniciar, solo esta sesión: sudo ./scripts/l6-g1-vfio-restore.sh)"
}

do_status() {
  echo "=== L6 G1 — estado del bind persistente (${FULL}) ==="
  echo ""
  echo "modprobe.d: $([[ -f "$MODPROBE_CONF" ]] && echo "presente" || echo "ausente")"
  [[ -f "$MODPROBE_CONF" ]] && grep -vE '^#' "$MODPROBE_CONF" | sed 's/^/  /'
  echo "modules-load.d: $([[ -f "$LOAD_CONF" ]] && echo "presente" || echo "ausente")"
  echo ""
  echo "cmdline activo:"
  if grep -q 'modprobe\.blacklist=' /proc/cmdline; then
    grep -o 'modprobe\.blacklist=[^ ]*' /proc/cmdline | sed 's/^/  /'
  else
    echo "  sin modprobe.blacklist (¿falta reiniciar tras --enable?)"
  fi
  echo ""
  echo "GRUB configurado:"
  grep -o 'modprobe\.blacklist=[^ "]*' "$GRUB" 2>/dev/null | sed 's/^/  /' || echo "  nada"
  echo ""
  echo "Drivers del slot ${slot}:"
  lspci -nnk -s "$slot" | sed 's/^/  /'
  echo ""
  echo "Grupos IOMMU: $(find /sys/kernel/iommu_groups -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)"
  if [[ -d /dev/vfio ]]; then
    echo "/dev/vfio: $(ls /dev/vfio 2>/dev/null | tr '\n' ' ')"
  fi
}

case "$action" in
  --enable)  do_enable ;;
  --disable) do_disable ;;
  --status)  do_status ;;
  *)
    echo "Uso: $0 [--enable|--disable|--status]" >&2
    exit 1
    ;;
esac
