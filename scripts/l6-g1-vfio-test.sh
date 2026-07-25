#!/usr/bin/env bash
# Post-reinicio: bind VFIO de la dGPU NVIDIA y prueba NV_PMC_BOOT_0 en soso.
#
# Requisito: la dGPU debe estar SIN driver propietario antes de ejecutar esto.
# En portátiles híbridos el panel va por la iGPU Intel, pero Xorg mantiene la dGPU
# abierta igual (PRIME render offload) y nvidia_drm posee fb0, así que primero:
#   TTY (Ctrl+Alt+F3): sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm
# Este script se niega a hacer unbind por sysfs de nvidia/nouveau: con el driver
# DRM vivo eso cuelga el kernel (GPF en drm_framebuffer_cleanup, 2026-07-24).
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
      # NUNCA hacer unbind por sysfs de un driver DRM vivo: con nvidia_drm cargado
      # el unbind provoca un GPF en drm_framebuffer_cleanup y cuelga la máquina
      # (2026-07-24, kernel 7.0.0-28 + nvidia 595.84). Hay que descargar la pila.
      if [[ "$cur" == nvidia* || "$cur" == "nouveau" ]]; then
        echo "" >&2
        echo "FAIL: ${fbdf} está en el driver DRM '${cur}'." >&2
        echo "      Un unbind por sysfs aquí cuelga el kernel (GPF en drm_framebuffer_cleanup)," >&2
        echo "      así que no se intenta. Libera la GPU descargando los módulos:" >&2
        echo "        TTY (Ctrl+Alt+F3): sudo ./scripts/l6-g1-nvidia-release.sh --stop-dm" >&2
        echo "      y vuelve a lanzar este script." >&2
        exit 1
      fi
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

# El timeout cubre compilación + arranque. Si el port nouveau se recompila desde cero
# 90s no bastan: precalienta con `cargo xtask build` o sube SOSO_G1_TIMEOUT.
TIMEOUT="${SOSO_G1_TIMEOUT:-90}"
echo "Lanzando soso (timeout ${TIMEOUT}s, log → ${log})..."
cd "$ROOT"
run_user="${SUDO_USER:-$USER}"
run_home=$(getent passwd "$run_user" | cut -d: -f6)
run_home="${run_home:-/home/${run_user}}"

# sudo resetea PATH (secure_path), así que `cargo` de ~/.cargo/bin no se encuentra.
# Resolvemos el binario y pasamos PATH/HOME explícitos (rustup necesita $HOME).
cargo_bin="${SOSO_CARGO:-}"
if [[ -z "$cargo_bin" ]]; then
  for cand in "${run_home}/.cargo/bin/cargo" /usr/local/bin/cargo /usr/bin/cargo; do
    [[ -x "$cand" ]] && { cargo_bin="$cand"; break; }
  done
fi
if [[ -z "$cargo_bin" ]]; then
  echo "FAIL: no encuentro 'cargo' para el usuario ${run_user}." >&2
  echo "      Indícalo con SOSO_CARGO=/ruta/a/cargo sudo $0" >&2
  exit 1
fi
echo "cargo: ${cargo_bin}"

# La clave con la que xtask autoriza el acceso: la del usuario si la tiene, y
# si no la de test que genera `client_pubkey()` (xtask/src/main.rs).
ssh_key="${run_home}/.ssh/id_ed25519"
[[ -f "$ssh_key" ]] || ssh_key="${ROOT}/target/soso_test_key"

soso_ssh() {
  sudo -u "$run_user" -- env "HOME=${run_home}" \
    ssh -i "$ssh_key" -p 2222 \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR -o ConnectTimeout=10 \
        soso@localhost "$@" </dev/null >/dev/null 2>&1
}

# NO se mata QEMU con `timeout`: cerrar el proceso con el GSP vivo hace que
# vfio-pci resetee una GPU que sigue ejecutando GSP-RM y haciendo DMA, y eso
# colgó el host entero el 2026-07-25 (sin dejar traza de panic — fue un lockup).
# En su lugar se espera al prompt, se pide `halt`, y soso apaga el GSP por el
# camino (SYS_HALT → gpu::shutdown → gsp_fini). El kill solo es el último
# recurso, y avisa de que se está haciendo lo peligroso.
sudo -u "$run_user" -- env \
  "PATH=$(dirname "$cargo_bin"):/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
  "HOME=${run_home}" \
  SOSO_QEMU_GPU="vfio:${BDF}" \
  "$cargo_bin" xtask run >"$log" 2>&1 &
run_pid=$!

booted=0
deadline=$((SECONDS + TIMEOUT))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  if grep -q 'sosh —' "$log" 2>/dev/null; then
    booted=1
    break
  fi
  kill -0 "$run_pid" 2>/dev/null || break
  sleep 1
done

if [[ "$booted" == 1 ]]; then
  echo "soso arriba — pidiendo halt por SSH para que apague el GSP"
  soso_ssh halt || true
else
  echo "AVISO: no se vio el prompt en ${TIMEOUT}s; no hay a quién pedirle el halt." >&2
fi

# Margen para el apagado ordenado: gsp_fini tiene dos esperas de 2 s.
for _ in $(seq 1 20); do
  kill -0 "$run_pid" 2>/dev/null || break
  sleep 1
done

if kill -0 "$run_pid" 2>/dev/null; then
  echo "" >&2
  echo "AVISO: soso no se apagó solo. Matando QEMU con el GSP posiblemente vivo —" >&2
  echo "       es justo el escenario que colgó el host el 2026-07-25. Si la máquina" >&2
  echo "       se congela aquí, mira si el log llegó a 'GSP-RM apagado'." >&2
  kill -TERM "$run_pid" 2>/dev/null || true
  sleep 3
  kill -KILL "$run_pid" 2>/dev/null || true
fi
wait "$run_pid" 2>/dev/null || true

if grep -q 'GSP-RM apagado' "$log"; then
  grep 'nouveau-lx: fini —\|GSP-RM apagado' "$log" || true
fi

if grep -q 'NV_PMC_BOOT_0=' "$log"; then
  grep 'nvidia:' "$log" || true
  echo ""
  echo "GO: NV_PMC_BOOT_0 legible desde soso."
  exit 0
fi

echo "FAIL o incompleto — revisar ${log}"
# Nota: `grep … | tail` siempre sale 0 (estado del último comando del pipe), así que
# el fallback hay que decidirlo mirando si hubo coincidencias, no por el exit code.
hits=$(grep -E 'nvidia:|gpu:|VFIO|error|fail|No existe|not found' "$log" | tail -20 || true)
if [[ -n "$hits" ]]; then
  echo "$hits"
else
  tail -20 "$log" || true
fi
exit 1
