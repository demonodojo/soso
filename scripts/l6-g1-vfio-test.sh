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

# ---- Root port PCIe (cap automático antes del FMC) ---------------------------
# Target Link Speed no sobrevive al reboot. Gen5 durante el reset del FMC bajo
# VFIO provoca tormenta AER en el root port (2026-07-27); Gen3 es el default
# estable. SOSO_G1_PCIE_GEN=0 no toca nada; SOSO_G1_PCIE_BUMP=4|5 sube la
# velocidad tras 'GSP-RM listo' (opt-in, ver bucle de arranque).
if [[ -n "${SOSO_G1_ROOT_PORT:-}" ]]; then
  RP_SYS="/sys/bus/pci/devices/0000:${SOSO_G1_ROOT_PORT}"
else
  RP_SYS=$(dirname "$(readlink -f "/sys/bus/pci/devices/${FULL}")")
fi
RP_BDF=$(basename "$RP_SYS")
RP_SHORT="${RP_BDF#0000:}"

pcie_set_target_link_speed() {
  local gen="$1" label="${2:-}"
  local readback cur
  if [[ -z "$gen" || "$gen" == "0" ]]; then
    return 0
  fi
  if ! command -v setpci >/dev/null; then
    echo "FAIL: setpci no encontrado; hace falta para cap PCIe Gen${gen}." >&2
    exit 1
  fi
  if ! setpci -s "$RP_SHORT" "CAP_EXP+0x30.w=${gen}:f" 2>/dev/null; then
    echo "FAIL: no se pudo escribir Target Link Speed Gen${gen} en ${RP_SHORT}." >&2
    exit 1
  fi
  readback=$(setpci -s "$RP_SHORT" CAP_EXP+0x30.w 2>/dev/null || echo "0")
  readback=$((readback & 0xf))
  if [[ "$readback" != "$gen" ]]; then
    echo "FAIL: Target Link Speed en ${RP_SHORT} = ${readback}, se pidió Gen${gen}." >&2
    exit 1
  fi
  if ! setpci -s "$RP_SHORT" CAP_EXP+0x10.w=20:20 2>/dev/null; then
    echo "FAIL: retrain del enlace en ${RP_SHORT} falló." >&2
    exit 1
  fi
  sleep 0.5
  cur=$(<"${RP_SYS}/current_link_speed" 2>/dev/null || echo "?")
  echo "pcie: ${label}Target Link Speed Gen${gen} en ${RP_SHORT} (current_link_speed=${cur})"
}

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
fifo="${ROOT}/target/.g1-vfio-serial.fifo"
mkdir -p "${ROOT}/target"
rm -f "$log" "$fifo"

# El timeout cubre compilación + arranque. Si el port nouveau se recompila desde cero
# no basta: precalienta con `cargo xtask build` o sube SOSO_G1_TIMEOUT.
# 90 s se quedaron cortos el 2026-07-25: el bring-up del GSP carga 63 MB de
# gsp-570.144.bin desde sosomfs antes de llegar al prompt, y al expirar el plazo
# el script mataba QEMU con el GSP vivo y colgaba el host.
TIMEOUT="${SOSO_G1_TIMEOUT:-180}"
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

# El sshd de soso NO atiende peticiones `exec`: sólo abre shell interactiva.
# Pasar el comando como argv daba `exec request failed on channel 0` y el halt
# no llegaba a ejecutarse nunca — el script acababa avisando de que el guest no
# se había apagado con el guest perfectamente vivo en su prompt (2026-07-27).
# El comando va por stdin, como hace el arnés de `cargo xtask test`.
#
# Y va EXACTAMENTE como lo hace ese arnés (xtask/src/test.rs), porque `printf | ssh`
# a secas perdía la carga entera sin decir nada (2026-07-28: el log de la carga
# tenía sólo el motd y el `$`, y sosh hace eco de lo que lee, así que no había
# llegado un solo byte). Dos diferencias, las dos necesarias:
#
#   -tt          el arnés fuerza pty; sin pedirla el canal es otro camino menos
#                probado del sshd propio.
#   stdin abierto  `printf | ssh` cierra stdin al instante y pega el CHANNEL_EOF
#                al mismo lote que los datos. En kernel/src/net/ssh.rs el EOF se
#                traga con `Err(ChannelEOF) => {}`, y si sunset lo entrega antes
#                de drenar lo que quedaba en el canal los bytes se pierden. El
#                `halt` (una línea, guest ya ocioso) colaba por temporización;
#                la carga, enviada cuando el guest aún estaba ocupado con el
#                bring-up, no. No se depende de esa carrera: no se manda EOF.
#
# El FIFO es lo que mantiene stdin abierto: el `sleep` de fondo tiene el extremo
# de escritura cogido, así que cuando `printf` cierra el suyo ssh NO ve EOF. La
# sesión termina porque el guest cierra el canal al salir la shell (`exit`/`halt`),
# no porque se le cierre la entrada.
#
# Y todo bajo `timeout`: antes NO había ninguno y un guest que no contestase
# colgaba el ciclo para siempre (2026-07-28, seis minutos hasta que se miró; con
# VFIO no se puede salir del paso matando QEMU).
soso_ssh_do() {
  local salida="$1" max="$2"; shift 2
  local in; in=$(mktemp -u "${ROOT}/target/.g1-ssh-in.XXXXXX")
  mkfifo "$in" || return 1
  chmod 666 "$in"
  # Ambos escritores bloquean en el open hasta que ssh abra para leer.
  sleep "$max" >"$in" &
  local holder=$!
  printf '%s\n' "$@" >"$in" &
  local writer=$!
  timeout -k 5 "$max" sudo -u "$run_user" -- env "HOME=${run_home}" \
    ssh -tt -i "$ssh_key" -p 2222 \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR -o ConnectTimeout=10 \
        soso@localhost <"$in" >"$salida" 2>&1
  local rc=$?
  kill "$holder" "$writer" 2>/dev/null || true
  wait "$holder" "$writer" 2>/dev/null || true
  rm -f "$in"
  # 124 = se agotó el `timeout`: el guest no cerró el canal. Lo dice, porque a
  # partir de aquí el `halt` puede no llegar y hay que soltar la GPU a mano.
  if [[ "$rc" == 124 ]]; then
    echo "AVISO: la sesión SSH no terminó en ${max}s (se corta y se sigue)." >&2
  fi
  return "$rc"
}

soso_ssh() {
  soso_ssh_do /dev/null "${SOSO_G1_SSH_TIMEOUT:-60}" "$@"
}

# Igual que `soso_ssh` pero guardando la salida: la carga de trabajo de G4f/G5 se
# lanza desde userspace y lo que dice (`on_gpu=1`, tok/s, matvec en el dispositivo)
# es justo el criterio GO, así que no se puede tirar a /dev/null.
soso_ssh_log() {
  local salida="$1"; shift
  soso_ssh_do "$salida" "${SOSO_G1_CMD_TIMEOUT:-300}" "$@"
}

# NO se mata QEMU con `timeout`: cerrar el proceso con el GSP vivo hace que
# vfio-pci resetee una GPU que sigue ejecutando GSP-RM y haciendo DMA, y eso
# colgó el host entero el 2026-07-25 (sin dejar traza de panic — fue un lockup).
# En su lugar se espera al prompt, se pide `halt`, y soso apaga el GSP por el
# camino (SYS_HALT → gpu::shutdown → gsp_fini). Si eso no ocurre, el script se
# rinde y deja QEMU vivo: matarlo solo pasa con SOSO_G1_FORCE_KILL=1.
#
# El log tiene que sobrevivir a un cuelgue del host. Con `>"$log"` la cola se
# queda en page cache y se pierde: el 2026-07-25 el fichero acabó con 1813 bytes
# a NUL —justo las ~18 líneas del bring-up que hacían falta para saber dónde se
# quedó—. Ahora la salida va por un FIFO a `dd oflag=dsync`, que sincroniza cada
# trozo antes de leer el siguiente. dd tiene que seguir vaciando el FIFO mientras
# QEMU viva, o el escritor se bloquearía al llenarse la tubería.
# Red contra el cuelgue de la FLR de cierre (2026-07-27, panic con traza en
# /var/crash/202607271829): al cerrar QEMU el fd de vfio,
# vfio_pci_core_disable() fuerza una FLR y pci_dev_wait() sondea el espacio de
# configuración por port IO CF8/CFC con pci_config_lock tomado e IRQs
# desactivadas. Si el enlace PCIe ya está muerto —y el COT lo mata: aquel día la
# tarjeta se cayó del bus a los 36 s de arrancar el guest— esa lectura no
# completa NUNCA: hard LOCKUP en pci_conf1_read+0xd5 y panic.
#
# Esto pasó con un apagado del guest PERFECTAMENTE ordenado (halt por SSH, soso
# cerró el GSP y salió solo), así que SOSO_G1_FORCE_KILL no protege de nada
# aquí: no depende de cómo muera QEMU, sino de que la tarjeta ya estuviera caída.
# Vaciar reset_method desactiva todos los métodos de reset del dispositivo
# (pci.c reset_method_store: sysfs_streq(buf,"") → reset_methods[0]=0), así que
# vfio no intenta la FLR al cerrar y el host sobrevive para contarlo.
#
# Al devolverlo, el valor exacto que había puede ser IRRECUPERABLE, y eso no es
# un fallo nuestro: 01:00.0 anuncia 'flr bus', pero escribir 'bus' da EINVAL
# ("Unsupported reset method 'bus'"). El probe de pci_reset_bus_function() exige
# que el dispositivo esté SOLO en su bus (pci_parent_bus_reset: cualquier vecino
# en bus->devices → -ENOTTY) y la función de audio 01:00.1 comparte el bus 01.
# Que 'bus' figure ahí es un artefacto del orden de enumeración: pci_device_add()
# llama a pci_init_capabilities() —y con ella a pci_init_reset_methods()— ANTES
# del list_add_tail() a bus->devices, así que al sondear 01:00.0 el bus estaba
# vacío y 'bus' pasó. No vuelve a pasar nunca. Por lo mismo 01:00.1 no tiene ni
# fichero reset_method: sin ningún método el atributo no se crea
# (pci_dev_reset_method_attr_is_visible → pci_reset_supported).
# Así que se intenta el valor exacto y, si el kernel lo rechaza, "default"
# (→ pci_init_reset_methods()), que aquí deja 'flr'. Eso NO es una re-derivación
# defectuosa: es la respuesta correcta para el bus tal como está ahora.
#
# Cap PCIe ANTES de la FLR: la FLR reentrena el enlace y debe hacerlo con el
# target ya fijado (Gen3 por defecto). SOSO_G1_PCIE_GEN=0 = no tocar.
SOSO_G1_PCIE_GEN="${SOSO_G1_PCIE_GEN:-3}"
if [[ "$SOSO_G1_PCIE_GEN" != "0" ]]; then
  pcie_set_target_link_speed "$SOSO_G1_PCIE_GEN" ""
fi

# ---- Reset DELIBERADO antes de arrancar (2026-07-29) -------------------------
#
# Vaciar reset_method protege al host de la FLR de CIERRE, pero también quita la
# de APERTURA: vfio recibe la tarjeta tal como la dejó el ciclo anterior, y sin
# reset el firmware de la propia GPU (GFW) no vuelve a correr su devinit. El
# 2026-07-29 eso se midió desde dentro: `0x118234 = 0x00000000` (progress 0, ni
# empezado) al mapear BAR0, y al mandarle el COT igual el FMC arrancó y **a los
# 313 ms la GPU se cayó del bus** con AER uncorrectable. O sea: un ciclo por
# reinicio del equipo.
#
# Así que la FLR se hace aquí, a propósito y con el enlace sano, que es
# exactamente lo que vfio haría si le dejáramos. El orden importa: primero
# resetear (necesita reset_method con métodos), después vaciarlo para el cierre.
#
# El peligro es el mismo de siempre —una FLR sobre un enlace muerto cuelga el
# host— así que hay puerta: sólo se resetea si el enlace del root port está a su
# velocidad plena y no hay AER reciente de la GPU ni de su puerto. Con
# SOSO_G1_NO_RESET=1 se salta (y entonces cuenta con llegar con la tarjeta recién
# arrancada).
gpu_link_healthy() {
  local rp speed err_pat bdf_pat parent
  rp=$(dirname "$(readlink -f "/sys/bus/pci/devices/${FULL}")")
  speed=$(<"${rp}/current_link_speed") 2>/dev/null || return 1
  # Un enlace entrenado a 2.5 GT/s es el síntoma de la caída (baja y no sube).
  [[ "$speed" == "2.5 GT/s PCIe" ]] && return 1
  err_pat='PCIe Bus Error|error message received|AER: (Multiple )?(Corrected|Correctable|Uncorrectable|Fatal|Non-Fatal)'
  bdf_pat="0000:${BDF}"
  parent=$(basename "$rp")
  [[ "$parent" == 0000:* ]] && bdf_pat="${bdf_pat}|${parent}"
  if dmesg 2>/dev/null | tail -200 | grep -E "$err_pat" | grep -qE "$bdf_pat"; then
    return 1
  fi
  return 0
}

if [[ "${SOSO_G1_NO_RESET:-0}" == "1" ]]; then
  echo "red: reset previo saltado (SOSO_G1_NO_RESET=1) — si el devinit no ha"
  echo "     corrido, soso se negará a mandar el COT y lo dirá"
elif [[ ! -w "/sys/bus/pci/devices/${FULL}/reset" ]]; then
  echo "AVISO: no hay fichero 'reset' escribible en ${FULL}: no se puede forzar el" >&2
  echo "       devinit. Si soso dice que el GFW no ha completado, reinicia." >&2
elif ! gpu_link_healthy; then
  echo "AVISO: el enlace no está sano (velocidad reducida o AER reciente): NO se" >&2
  echo "       intenta la FLR, porque sobre un enlace muerto cuelga el host." >&2
  echo "       Reinicia para devolver la tarjeta a un estado arrancable." >&2
else
  echo "red: reset de función a ${FULL} para que el GFW rehaga el devinit…"
  if printf '1\n' >"/sys/bus/pci/devices/${FULL}/reset" 2>/dev/null; then
    # El GFW tarda del orden de cientos de ms; soso lo espera otra vez por su
    # cuenta (hasta 2050 ms, los de tu102_devinit_wait), así que aquí basta con
    # no adelantarse al re-entrenado del enlace.
    sleep 1
    echo "red: reset hecho; enlace: $(<"$(dirname "$(readlink -f "/sys/bus/pci/devices/${FULL}")")/current_link_speed")"
  else
    echo "AVISO: la FLR falló; sigue el ciclo, pero si el GFW no ha completado soso" >&2
    echo "       se negará a arrancar el GSP." >&2
  fi
fi

declare -A saved_reset=()
disabled_reset=()
for path in /sys/bus/pci/devices/0000:${slot}.*; do
  [[ -w "${path}/reset_method" ]] || continue
  prev=$(<"${path}/reset_method")
  # Un write de 0 bytes puede no llegar al store handler; hay que mandar el "\n",
  # que sysfs_streq() recorta antes de comparar con "".
  printf '\n' >"${path}/reset_method" 2>/dev/null || true
  if [[ -z "$(<"${path}/reset_method")" ]]; then
    disabled_reset+=("$path")
    [[ -n "$prev" ]] && saved_reset["$path"]="$prev"
    echo "red: $(basename "$path") reset_method '${prev:-(ya vacío)}' → (ninguno)"
  else
    echo "AVISO: no se pudo desactivar reset_method en $(basename "$path");" >&2
    echo "       si la GPU se cae del bus, la FLR de cierre puede colgar el host." >&2
  fi
done

# Devolver reset_method sólo si la tarjeta sigue en el bus. Para decidirlo NO se
# lee el espacio de configuración desde el host: esa lectura es exactamente la
# que se cuelga si el enlace está muerto. Las fuentes son el propio guest, que
# lo registra, y los AER del root port.
#
# El AER hay que buscarlo también en el ROOT PORT, no sólo en el BDF de la GPU:
# el 2026-07-27 la tormenta fue casi toda del puerto (00:06.0, "Data Link Layer",
# Rollover+Timeout) y sólo UNA línea llevaba el 01:00.0. Aquí conviene pasarse de
# prudente: un falso positivo cuesta un reboot, un falso negativo cuelga el host.
restore_reset_method() {
  local path parent err_pat bdf_pat
  # Una línea cuenta como daño si lleva firma de ERROR *y* menciona la GPU o su
  # puerto. Sin la firma, `AER.*<bdf>` daría positivo con el inofensivo
  # "pcieport 0000:00:06.0: AER: enabled with IRQ 124" de cualquier arranque sano.
  err_pat='PCIe Bus Error|error message received|AER: (Multiple )?(Corrected|Correctable|Uncorrectable|Fatal|Non-Fatal)'
  bdf_pat="0000:${BDF}"
  parent=$(basename "$(dirname "$(readlink -f "/sys/bus/pci/devices/${FULL}")")")
  [[ "$parent" == 0000:* ]] && bdf_pat="${bdf_pat}|${parent}"
  if grep -q 'GPU fuera del bus\|status=gone' "$log" 2>/dev/null ||
     dmesg 2>/dev/null | grep -E "$err_pat" | grep -qE "$bdf_pat"; then
    echo "" >&2
    echo "AVISO: la dGPU se cayó del bus durante la prueba (lo dice el log del guest" >&2
    echo "       o hay AER del root port). reset_method se queda DESACTIVADO: una FLR" >&2
    echo "       sobre un enlace muerto cuelga el host sin remedio ni traza útil." >&2
    echo "       La tarjeta no vuelve sana sin un ciclo de alimentación — 'sudo reboot'" >&2
    echo "       la devuelve a 32 GT/s x8 (comprobado el 2026-07-27)." >&2
    return
  fi
  for path in "${disabled_reset[@]}"; do
    local want="${saved_reset[$path]:-default}" cand wrote="" now
    # Escribir reset_method no resetea nada y el probe de 'flr' mira devcap
    # cacheado, sin tocar el espacio de configuración: es seguro incluso si la
    # tarjeta estuviera muda (que aquí ya sabemos que no lo está).
    for cand in "$want" default; do
      printf '%s\n' "$cand" >"${path}/reset_method" 2>/dev/null || continue
      wrote="$cand"
      break
    done
    if [[ -z "$wrote" ]]; then
      echo "AVISO: no se pudo restaurar reset_method en $(basename "$path"): queda" >&2
      echo "       DESACTIVADO hasta el próximo reinicio, o sea que ni vfio ni el" >&2
      echo "       driver nvidia podrán resetear la GPU (pci_reset_function → -ENOTTY)." >&2
      continue
    fi
    now=$(<"${path}/reset_method")
    echo "red: $(basename "$path") reset_method restaurado (${now})"
    if [[ "$wrote" == "default" && "$want" != "default" ]]; then
      echo "nota: '${want}' no se pudo reponer tal cual (el kernel rechaza 'bus' con la" >&2
      echo "      función de audio en el mismo bus); 'default' dejó '${now}'." >&2
    fi
  done
}

# ---- Muestreo del enlace PCIe durante la prueba ------------------------------
#
# Las tres caídas de la tarjeta (2026-07-27 18:28, 2026-07-28 02:15 y 02:44)
# tienen la misma firma en el host: AER correctables de capa de enlace
# (Rollover + Timeout) en el root port `00:06.0` y un `Uncorrectable (Non-Fatal)`
# de `01:00.0`, siempre ~1 s después de que el FMC empiece a ejecutar. Con eso
# solo no se puede decir si el enlace se cae y por eso la GPU calla, o si la GPU
# se cuelga y el enlace es la consecuencia — y son diagnósticos opuestos.
#
# El root port **no está en passthrough**, así que su lado del enlace se puede
# leer desde el host mientras corre la prueba sin tocar nada de la GPU. Se
# muestrea velocidad y anchura cada 200 ms con marca de tiempo, y así el momento
# exacto del cambio se puede alinear contra el log de serie del guest.
#
# La GPU NO se sondea: leer su espacio de configuración con el enlace agonizando
# es justo lo que colgó el host el 27 (`pci_conf1_read` con las IRQs cerradas).
linklog="${ROOT}/target/g1-vfio-link.log"
rm -f "$linklog"
link_pid=""
if [[ -r "${RP_SYS}/current_link_speed" ]]; then
  {
    prev=""
    while :; do
      now=$(cat "${RP_SYS}/current_link_speed" 2>/dev/null)/$(cat "${RP_SYS}/current_link_width" 2>/dev/null)
      if [[ "$now" != "$prev" ]]; then
        printf '%s  root port %s → %s\n' "$(date +%H:%M:%S.%3N)" "$RP_SHORT" "$now"
        prev="$now"
      fi
      sleep 0.2
    done
  } >"$linklog" 2>&1 &
  link_pid=$!
fi

mkfifo -m 666 "$fifo"
dd of="$log" bs=4096 oflag=dsync status=none <"$fifo" &
dd_pid=$!

# sudo -u + env_reset tira SOSO_*; reinyectar las que el ciclo necesite.
pass_env=(
  "PATH=$(dirname "$cargo_bin"):/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  "HOME=${run_home}"
  "SOSO_QEMU_GPU=vfio:${BDF}"
)
for v in SOSO_MODELS_DIR SOSO_MODELS_SIZE SOSO_QEMU_MEM SOSO_LXDDE SOSO_LXDDE_MODE; do
  if [[ -n "${!v:-}" ]]; then
    pass_env+=("${v}=${!v}")
  fi
done
# VFIO fija (mlock) toda la RAM del guest. El hard limit del usuario (~3.7 GiB
# aquí) hace fallar -m 4G+ con VFIO_MAP_DMA; con -m 2G el guest hace OOM en
# `bench`. Subimos memlock como root y luego bajamos a $run_user (el RLIMIT
# sobrevive al setuid).
uid=$(id -u "$run_user")
gid=$(id -g "$run_user")
if command -v setpriv >/dev/null && command -v prlimit >/dev/null; then
  prlimit --memlock=unlimited:unlimited -- \
    setpriv --reuid="$uid" --regid="$gid" --init-groups -- \
    env "${pass_env[@]}" \
    "$cargo_bin" xtask run >"$fifo" 2>&1 &
else
  echo "AVISO: sin setpriv/prlimit; memlock del usuario puede tumbar VFIO con -m>2G" >&2
  sudo -u "$run_user" -- env "${pass_env[@]}" \
    "$cargo_bin" xtask run >"$fifo" 2>&1 &
fi
run_pid=$!

booted=0
pcie_bumped=0
deadline=$((SECONDS + TIMEOUT))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  # Bump opt-in tras GSP-RM listo: la ventana peligrosa es el FMC (~1 s tras COT).
  # Probar primero SOSO_G1_PCIE_BUMP=4; Gen5 solo si Gen4 aguanta varios ciclos.
  if [[ "$pcie_bumped" == 0 && -n "${SOSO_G1_PCIE_BUMP:-}" && "${SOSO_G1_PCIE_BUMP}" != "0" ]]; then
    if grep -q 'GSP-RM listo (RPC en marcha)' "$log" 2>/dev/null; then
      pcie_bumped=1
      pcie_set_target_link_speed "${SOSO_G1_PCIE_BUMP}" "bump post-GSP "
      printf '%s  host: bump Gen%s tras GSP-RM listo\n' "$(date +%H:%M:%S.%3N)" "${SOSO_G1_PCIE_BUMP}" \
        >>"$linklog" 2>/dev/null || true
    fi
  fi
  if grep -q 'sosh —' "$log" 2>/dev/null; then
    booted=1
    break
  fi
  kill -0 "$run_pid" 2>/dev/null || break
  sleep 1
done

if [[ "$booted" == 1 ]]; then
  # Carga de trabajo ANTES del halt. El bring-up deja el compute armado pero no
  # lanza ningún kernel a propósito (un QMD que falle en el arranque deja la
  # tarjeta en un estado del que sólo se sale reseteando el equipo), así que sin
  # esto el ciclo probaría G4e y el armado de G4f, pero ni saxpy ni matvec. Y un
  # ciclo de VFIO cuesta cerrar la sesión gráfica: conviene que pruebe todo.
  if [[ -n "${SOSO_G1_CMD:-}" ]]; then
    echo "soso arriba — carga de trabajo por SSH:"
    printf '  %s\n' "${SOSO_G1_CMD}"
    cmdlog="${log%.log}-cmd.log"
    soso_ssh_log "$cmdlog" "${SOSO_G1_CMD}" "exit" || true
    echo "--- salida de la carga (${cmdlog}) ---"
    # La pty de `-tt` mete \r en cada salto: se quitan para leer el log.
    tr -d '\r' <"$cmdlog" | sed 's/^/  /' || true
    echo "---"
    # sosh hace eco de lo que lee, así que si el comando no aparece en su propia
    # salida es que el guest no lo recibió — el fallo silencioso del 2026-07-28,
    # que sólo se notaba porque el ciclo se quedaba colgado sin decir nada.
    primer=${SOSO_G1_CMD%%$'\n'*}
    if ! tr -d '\r' <"$cmdlog" | grep -qF -- "$primer"; then
      echo "AVISO: la carga NO llegó al guest (sosh no hizo eco de '${primer}')." >&2
      echo "       El ciclo probó el bring-up, pero ni saxpy ni matvec." >&2
    fi
  fi
  echo "pidiendo halt por SSH para que apague el GSP"
  soso_ssh halt || true
else
  echo "AVISO: no se vio el prompt en ${TIMEOUT}s; no hay a quién pedirle el halt." >&2
fi

# Margen para el apagado ordenado: gsp_fini tiene dos esperas de 2 s.
for _ in $(seq 1 20); do
  kill -0 "$run_pid" 2>/dev/null || break
  sleep 1
done

qemu_vivo=0
if kill -0 "$run_pid" 2>/dev/null; then
  if [[ "${SOSO_G1_FORCE_KILL:-0}" == 1 ]]; then
    echo "" >&2
    echo "SOSO_G1_FORCE_KILL=1 — matando QEMU con el GSP posiblemente vivo. Es justo" >&2
    echo "       el escenario que colgó el host dos veces el 2026-07-25. Si la máquina" >&2
    echo "       se congela aquí, mira si el log llegó a 'GSP-RM apagado'." >&2
    kill -TERM "$run_pid" 2>/dev/null || true
    sleep 3
    kill -KILL "$run_pid" 2>/dev/null || true
  else
    qemu_vivo=1
    echo "" >&2
    echo "AVISO: soso no se apagó solo, y NO se mata QEMU: resetear por vfio-pci una" >&2
    echo "       GPU que sigue ejecutando GSP-RM y haciendo DMA congela el host entero" >&2
    echo "       (2026-07-25, dos veces, sin dejar traza de panic). QEMU sigue vivo en" >&2
    echo "       pid ${run_pid} y el log sigue creciendo en ${log}." >&2
    echo "       Salidas, por orden de preferencia:" >&2
    echo "         1) ssh -p 2222 soso@localhost halt   — si el guest responde, apaga" >&2
    echo "            el GSP por gsp_fini y es la única salida limpia" >&2
    echo "         2) si el guest está colgado: 'sudo reboot' del host. Un reinicio" >&2
    echo "            ordenado es mucho mejor que congelarse y tener que resetear" >&2
    echo "         3) SOSO_G1_FORCE_KILL=1 sudo $0 — mata, asumiendo el cuelgue" >&2
  fi
fi

if [[ "$qemu_vivo" == 0 ]]; then
  wait "$run_pid" 2>/dev/null || true
  # dd sale por EOF cuando se cierra el último extremo de escritura del FIFO.
  wait "$dd_pid" 2>/dev/null || true
  rm -f "$fifo"
  # QEMU ya cerró el fd de vfio: la FLR peligrosa, si tocaba, ya se saltó.
  if [[ ${#disabled_reset[@]} -gt 0 ]]; then
    restore_reset_method
  fi
else
  echo "       reset_method sigue desactivado mientras QEMU viva: el peligro está en" >&2
  echo "       el cierre del fd, no ahora. Para rearmarlo cuando la GPU esté sana:" >&2
  echo "         echo default | sudo tee /sys/bus/pci/devices/${FULL}/reset_method" >&2
fi

if [[ -n "$link_pid" ]]; then
  kill "$link_pid" 2>/dev/null || true
  wait "$link_pid" 2>/dev/null || true
fi

if grep -q 'GSP-RM apagado' "$log"; then
  grep 'nouveau-lx: fini —\|GSP-RM apagado' "$log" || true
fi

# El enlace, si dijo algo. Una sola línea = nunca cambió de estado: la GPU se
# calló con el enlace entrenado, y entonces el problema NO es el enlace sino la
# tarjeta. Varias líneas = el enlace se cayó o renegoció, y el orden respecto al
# log de serie dice quién arrastró a quién.
if [[ -s "$linklog" ]]; then
  n=$(wc -l <"$linklog")
  if [[ "$n" -le 1 ]]; then
    echo "enlace: sin cambios durante la prueba ($(tail -1 "$linklog" | sed 's/.*→ //'))"
  else
    echo "enlace: ${n} cambios de estado — ${linklog}"
    cat "$linklog"
  fi
fi

# El criterio NO puede ser "aparece la cadena NV_PMC_BOOT_0=": con la tarjeta
# caída del bus el registro se lee 0xffffffff y esto cantaba GO igual
# (2026-07-28, se cayó arrancando el FMC y el script dio la prueba por buena).
# Un all-ones no es una lectura, es silencio — la misma trampa que fsp_lx.c ya
# documenta para el MMIO, aquí sin aplicar.
boot0=$(grep -oE 'NV_PMC_BOOT_0=0x[0-9a-fA-F]+' "$log" | tail -1 | cut -d= -f2)
if grep -q 'GPU fuera del bus\|se ha caído del bus' "$log"; then
  echo ""
  echo "FAIL: la GPU se cayó del bus durante la prueba (lo dice el log del guest)." >&2
  echo "      NV_PMC_BOOT_0=${boot0:-(sin lectura)} no cuenta: con el enlace muerto" >&2
  echo "      todo el espacio de configuración se lee a unos." >&2
  echo "      La tarjeta no vuelve sana sin ciclo de alimentación: 'sudo reboot'." >&2
  grep -E 'fuera del bus|caído del bus' "$log" | tail -5 >&2
  exit 1
fi
if [[ -n "$boot0" ]] && (( boot0 != 0xffffffff && boot0 != 0 )); then
  grep 'nvidia:' "$log" || true
  echo ""
  echo "GO: NV_PMC_BOOT_0 legible desde soso (${boot0})."
  exit 0
fi
if [[ -n "$boot0" ]]; then
  echo ""
  echo "FAIL: NV_PMC_BOOT_0=${boot0} no es una lectura válida." >&2
  exit 1
fi

echo "FAIL o incompleto — revisar ${log}"
if [[ "$qemu_vivo" == 1 ]]; then
  echo "      (QEMU sigue vivo en pid ${run_pid}: el log puede crecer todavía)"
fi
# Nota: `grep … | tail` siempre sale 0 (estado del último comando del pipe), así que
# el fallback hay que decidirlo mirando si hubo coincidencias, no por el exit code.
hits=$(grep -E 'nvidia:|gpu:|VFIO|error|fail|No existe|not found' "$log" | tail -20 || true)
if [[ -n "$hits" ]]; then
  echo "$hits"
else
  tail -20 "$log" || true
fi
exit 1
