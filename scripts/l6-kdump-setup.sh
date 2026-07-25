#!/usr/bin/env bash
# L6 — captura de crash del HOST para las pruebas de GPU.
#
# Por qué: el 2026-07-25 una prueba VFIO congeló la máquina entera y **no quedó
# ni una traza**. No fue un panic — `efi_pstore` está registrado en este equipo y
# capturó el GPF de `drm_framebuffer_cleanup` del día 24, pero de aquel cuelgue
# el journal se corta en seco y pstore está vacío. Sin evidencia, el siguiente
# ciclo de GPU vuelve a ser adivinar.
#
# LO IMPORTANTE, Y NO ES kdump: kdump captura *panics*. Un cuelgue duro no lo es,
# así que kdump por sí solo no habría capturado nada. Lo que convierte el cuelgue
# en algo capturable es `kernel.hardlockup_panic=1`: el detector NMI ya está
# activo (`nmi_watchdog=1`) pero hoy solo avisa; con esto, una CPU atascada ≥10 s
# con las interrupciones cerradas —que es justo la pinta de un reset de vfio-pci
# que no completa— provoca un panic, y ahí sí entra kdump.
#
# Así que después de esto hay tres canales, y los tres dicen algo:
#   1. kdump  → vmcore completo en /var/crash (lo mejor)
#   2. pstore → el final del dmesg, aunque kexec falle (ya funciona hoy)
#   3. nada de nada → **también es información**: descarta el lockup de software
#      y apunta a algo de nivel máquina (machine check, error fatal de PCIe,
#      reset de plataforma), que ningún software puede capturar.
#
# OJO, LECCIÓN DEL SELFTEST DE LAS 18:03 DEL 2026-07-25: los canales 1 y 2 no
# son independientes por defecto, son EXCLUYENTES. En panic() el orden es
# notificadores → __crash_kexec() → kmsg_dump(KMSG_DUMP_PANIC), y es kmsg_dump
# el que escribe pstore. Con un kernel de captura cargado, el control se va en
# __crash_kexec() y nunca se llega a pstore. Aquel selftest provocó el panic de
# verdad (el journal se corta en seco a las 18:03:54, justo tras el sudo) y dejó
# CERO evidencia en ambos sitios: ni vmcore, ni registro nuevo en pstore. O sea
# que activar kdump había tapado el único canal que ya estaba demostrado en este
# equipo. Lo arregla `crash_kexec_post_notifiers` (paso 5 de do_enable).
#
# Uso:
#   sudo ./scripts/l6-kdump-setup.sh --enable    → instalar + configurar, luego reiniciar
#   sudo ./scripts/l6-kdump-setup.sh --disable   → deshacer, luego reiniciar
#   ./scripts/l6-kdump-setup.sh --status         → qué hay puesto ahora (no toca nada)
#
# Precio: reserva ~512 MiB de RAM de forma permanente (de 30 GiB), y a partir de
# ahora un hard lockup **reinicia la máquina** en vez de quedarse colgado. Para
# depurar es lo que se quiere; si molesta, `--disable`.
set -euo pipefail

GRUBD=/etc/default/grub.d
GRUB_SNIPPET="${GRUBD}/soso-l6-kdump.cfg"
SYSCTL=/etc/sysctl.d/99-soso-l6-lockup.conf
KDUMP_DEFAULT=/etc/default/kdump-tools
POST_NOTIF=/sys/module/kernel/parameters/crash_kexec_post_notifiers
# Marcas para poder quitar limpiamente nuestro bloque de /etc/default/kdump-tools.
CMDLINE_MARK_BEGIN='# >>> soso L6 — cmdline del kernel de captura'
CMDLINE_MARK_END='# <<< soso L6'

action="${1:---status}"

need_root() {
  if [[ $(id -u) -ne 0 ]]; then
    echo "Ejecutar: sudo $0 ${action}" >&2
    exit 1
  fi
}

# --- status -------------------------------------------------------------------

do_status() {
  echo "=== L6 — captura de crash del host ==="

  local cmdline crash_size
  cmdline=$(cat /proc/cmdline)
  # grep -o suelta una línea por coincidencia y aquí hay dos crashkernel= (el de
  # la postinst de kdump-tools y el nuestro); gana el último, que es lo que
  # confirma kexec_crash_size más abajo.
  if [[ "$cmdline" == *crashkernel=* ]]; then
    echo "OK    crashkernel en el cmdline: $(grep -o 'crashkernel=[^ ]*' /proc/cmdline | tail -1)"
  else
    echo "FALTA crashkernel en el cmdline (¿configurado pero sin reiniciar?)"
  fi

  crash_size=$(cat /sys/kernel/kexec_crash_size 2>/dev/null || echo 0)
  if [[ "$crash_size" -gt 0 ]]; then
    echo "OK    memoria reservada: $((crash_size / 1024 / 1024)) MiB"
  else
    echo "FALTA memoria de crash reservada (kexec_crash_size = 0)"
  fi

  # "Cargado" no es "funciona": el 2026-07-25 estaba cargado y el selftest no
  # dejó vmcore. Cargado solo significa que el panic saltará a él.
  if [[ "$(cat /sys/kernel/kexec_crash_loaded 2>/dev/null || echo 0)" == "1" ]]; then
    echo "OK    kernel de captura CARGADO — el panic saltará ahí (probar: --selftest)"
  else
    echo "FALTA kernel de captura sin cargar (kdump-tools no activo)"
  fi

  # El que de verdad decide si el cuelgue de la GPU se convierte en capturable.
  local hl
  hl=$(sysctl -n kernel.hardlockup_panic 2>/dev/null || echo "?")
  if [[ "$hl" == "1" ]]; then
    echo "OK    hardlockup_panic=1 — un cuelgue duro se convierte en panic"
  else
    echo "FALTA hardlockup_panic=${hl} — un cuelgue duro NO dejará nada"
  fi
  echo "      nmi_watchdog=$(sysctl -n kernel.nmi_watchdog 2>/dev/null || echo '?') " \
       "softlockup_panic=$(sysctl -n kernel.softlockup_panic 2>/dev/null || echo '?') " \
       "panic_on_io_nmi=$(sysctl -n kernel.panic_on_io_nmi 2>/dev/null || echo '?')"

  # El que decide si pstore sigue siendo un canal vivo o queda tapado por kdump.
  local pn
  pn=$(cat "$POST_NOTIF" 2>/dev/null || echo '?')
  if [[ "$pn" == "Y" ]]; then
    echo "OK    crash_kexec_post_notifiers=Y — pstore escribe ANTES del kexec"
  else
    echo "FALTA crash_kexec_post_notifiers=${pn} — con kdump cargado, pstore no"
    echo "      llega a escribir: el kexec se lleva el control antes. Si el kernel"
    echo "      de captura se cuelga, no queda NADA (pasó el 2026-07-25 18:03)."
  fi

  # panic=0 significa que un fallo del camino de captura deja la máquina muerta.
  local pt
  pt=$(sysctl -n kernel.panic 2>/dev/null || echo '?')
  if [[ "$pt" != "0" ]]; then
    echo "OK    kernel.panic=${pt} — si la captura falla, reinicia en vez de colgarse"
  else
    echo "FALTA kernel.panic=0 — un panic sin captura deja la máquina parada para"
    echo "      siempre, y no se distingue del cuelgue que estabas investigando"
  fi

  echo "      cmdline del kernel de captura:"
  local capline
  capline=$(grep -m1 '^KDUMP_CMDLINE=' "$KDUMP_DEFAULT" 2>/dev/null || true)
  if [[ -n "$capline" ]]; then
    echo "        ${capline#KDUMP_CMDLINE=}"
  else
    echo "        (por defecto = /proc/cmdline, que aquí arrastra 'quiet splash'"
    echo "         y vt.handoff=7: si se cuelga, se cuelga sin decir nada)"
  fi

  local svc
  svc=$(systemctl is-enabled kdump-tools.service 2>/dev/null || true)
  echo "      servicio kdump-tools: ${svc:-(no instalado)}"

  echo "      pstore (segundo canal, independiente de kdump):"
  if [[ -d /var/lib/systemd/pstore ]] && [[ -n "$(ls -A /var/lib/systemd/pstore 2>/dev/null)" ]]; then
    ls -1 /var/lib/systemd/pstore | sed 's/^/        registro /'
  else
    echo "        (vacío)"
  fi

  # kdump escribe directorios <fecha>/dump.N. Los ficheros .crash sueltos que ya
  # hay ahí son de apport (cuelgues de programas de usuario) y no pintan nada.
  echo "      vmcore en /var/crash:"
  local dumps
  dumps=$(find /var/crash -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort || true)
  if [[ -n "$dumps" ]]; then
    echo "$dumps" | sed 's/^/        /'
  else
    echo "        (ninguno — los .crash sueltos de ahí son de apport, no de kdump)"
  fi
}

# --- enable -------------------------------------------------------------------

do_enable() {
  need_root
  echo "=== Activando captura de crash ==="

  # 1) Paquetes. kdump-tools arrastra kexec-tools y makedumpfile, y su postinst
  #    ya añade su propio crashkernel= por /etc/default/grub.d. Aun así ponemos
  #    el nuestro (paso 3) por si ese mecanismo cambia: `crashkernel=` repetido
  #    no es un problema, gana el último y el valor es el mismo orden.
  if ! dpkg -l kdump-tools 2>/dev/null | grep -q '^ii'; then
    echo "Instalando kdump-tools…"
    DEBIAN_FRONTEND=noninteractive apt-get install -y kdump-tools
  else
    echo "kdump-tools ya instalado"
  fi

  # 2) Que el servicio esté activo de verdad (el debconf puede dejarlo en no).
  if [[ -f "$KDUMP_DEFAULT" ]]; then
    if grep -q '^USE_KDUMP=' "$KDUMP_DEFAULT"; then
      sed -i 's/^USE_KDUMP=.*/USE_KDUMP=1/' "$KDUMP_DEFAULT"
    else
      echo 'USE_KDUMP=1' >>"$KDUMP_DEFAULT"
    fi
    echo "OK: USE_KDUMP=1 en ${KDUMP_DEFAULT}"
  fi
  systemctl enable kdump-tools.service >/dev/null 2>&1 || true

  # 3) crashkernel por /etc/default/grub.d, NO editando GRUB_CMDLINE_LINUX_DEFAULT.
  #    En esta máquina esa línea lleva el `modprobe.blacklist=` que pone
  #    l6-g1-vfio-persist.sh, y no hay por qué arriesgarse a tocarla: Ubuntu lee
  #    los .cfg de este directorio y los añade encima.
  mkdir -p "$GRUBD"
  cat >"$GRUB_SNIPPET" <<'EOF'
# soso L6 — reserva para el kernel de captura de kdump.
# Puesto por scripts/l6-kdump-setup.sh --enable; quitar con --disable.
# Deliberadamente por aquí y no en GRUB_CMDLINE_LINUX_DEFAULT: esa línea lleva
# el modprobe.blacklist del passthrough VFIO y no se toca.
# crash_kexec_post_notifiers invierte el orden dentro de panic(): kmsg_dump
# (= pstore) ANTES del kexec, en vez de después. Sin esto, cargar el kernel de
# captura deja pstore sin usar y un kexec que se cuelgue no deja rastro alguno
# (pasó en el selftest del 2026-07-25 18:03). En este kernel no hay sysctl para
# él —/proc/sys/kernel/crash_kexec_post_notifiers no existe—, solo el parámetro
# de arranque y /sys/module/kernel/parameters/, así que va aquí.
# El precio está documentado: los notificadores corren en un kernel ya roto y
# podrían colgarse antes de llegar al kexec. Se acepta a propósito: pstore es el
# canal demostrado en este equipo y kdump es el que está sin probar.
GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT crashkernel=2G-4G:320M,4G-32G:512M,32G-64G:1024M,64G-:2048M crash_kexec_post_notifiers"
EOF
  echo "OK: ${GRUB_SNIPPET}"

  # En caliente también, que este parámetro se puede escribir en runtime.
  echo Y >/sys/module/kernel/parameters/crash_kexec_post_notifiers 2>/dev/null \
    && echo "OK: crash_kexec_post_notifiers=Y ya activo (sin esperar al reinicio)" \
    || echo "AVISO: no se pudo poner crash_kexec_post_notifiers en caliente"

  # 4) Los sysctl que convierten un cuelgue en un panic capturable.
  cat >"$SYSCTL" <<'EOF'
# soso L6 — convertir cuelgues en panics para que kdump los capture.
# Puesto por scripts/l6-kdump-setup.sh --enable.
#
# El motivo está en el cuelgue del 2026-07-25: la máquina se congeló soltando la
# GPU y no dejó traza porque nunca llegó a hacer panic. kdump solo captura
# panics, así que sin esto kdump no sirve para este fallo.

# El grande: CPU atascada >=10 s con interrupciones cerradas (detector NMI, que
# ya estaba activo pero solo avisaba). Es la pinta de un reset de PCIe que no
# completa, que es la hipótesis principal del cuelgue.
kernel.hardlockup_panic = 1

# Un error de PCIe (SERR/paridad) llega como NMI de E/S en muchos chipsets, y
# "la GPU se cae del bus" es un final plausible de este experimento.
kernel.panic_on_io_nmi = 1

# CPU atascada en el kernel con las interrupciones abiertas. Umbral 20 s por
# defecto, muy por encima de las esperas de 2 s de gsp_fini, así que no debería
# dar falsos positivos. Si alguno aparece, esta es la primera línea a quitar.
kernel.softlockup_panic = 1

# NO se activa kernel.unknown_nmi_panic: en portátiles el firmware manda NMIs
# por sus cosas y es la fuente clásica de panics espurios. Si tras un cuelgue
# sin traza sospechas de un NMI de hardware, actívalo a mano y repite:
#   sudo sysctl -w kernel.unknown_nmi_panic=1

# Reiniciar solo a los 2 min en vez de quedarse muerta. Estaba en 0, y por eso
# el selftest del 2026-07-25 dejó la máquina parada sin reboot: cuando el camino
# de captura falla, panic=0 es un cuelgue indistinguible del fallo original.
# 120 s da tiempo a leer/fotografiar la pantalla antes de que se vaya.
# (No cubre el caso "el kernel de captura arranca y se cuelga": ahí ya estamos
# en otro kernel y este timeout no existe. Para eso está pstore.)
kernel.panic = 120
EOF
  sysctl -q -p "$SYSCTL"
  echo "OK: ${SYSCTL} (aplicado ya, sin esperar al reinicio)"

  # 5) Que el kernel de captura HABLE si se cuelga.
  #    kdump-config compone su cmdline como /proc/cmdline menos crashkernel=,
  #    más KDUMP_CMDLINE_APPEND. Aquí eso arrastra `quiet splash vt.handoff=7`,
  #    así que un cuelgue del kernel de captura es completamente silencioso —
  #    exactamente lo que se vio el 2026-07-25: pantalla congelada, sin reboot,
  #    sin una línea. Fijando KDUMP_CMDLINE damos una cmdline mínima y locuaz
  #    (kdump-config le añade encima su APPEND: reset_devices, nr_cpus=1,
  #    irqpoll, usbcore.nousb y systemd.unit=kdump-tools-dump.service).
  #
  #    Qué lleva y por qué:
  #      nomodeset  → nada de i915/nouveau; simpledrm ya da consola por el
  #                   framebuffer que EFI pasa a través del kexec.
  #      loglevel=7 console=tty0 → los mensajes en pantalla, sin quiet.
  #      modprobe.blacklist=vfio_* → el initrd de kdump lleva dentro el
  #                   soso-l6-vfio.conf, o sea que el kernel de captura intenta
  #                   enganchar vfio-pci a la dGPU recién reventada, y con
  #                   reset_devices activo. Para volcar memoria no hace falta.
  local root_spec
  root_spec=$(grep -o 'root=[^ ]*' /proc/cmdline || true)
  if [[ -z "$root_spec" ]]; then
    echo "AVISO: no encuentro root= en /proc/cmdline; dejo KDUMP_CMDLINE sin tocar" >&2
  else
    local cap_cmdline="${root_spec} ro nomodeset loglevel=7 console=tty0"
    cap_cmdline+=" modprobe.blacklist=vfio_pci,vfio_iommu_type1,nvidia,nvidia_drm,nvidia_modeset,nvidia_uvm,nouveau"
    sed -i "/^${CMDLINE_MARK_BEGIN}$/,/^${CMDLINE_MARK_END}$/d" "$KDUMP_DEFAULT"
    {
      echo "$CMDLINE_MARK_BEGIN"
      echo "# Puesto por scripts/l6-kdump-setup.sh --enable; se quita con --disable."
      echo "KDUMP_CMDLINE=\"${cap_cmdline}\""
      echo "$CMDLINE_MARK_END"
    } >>"$KDUMP_DEFAULT"
    echo "OK: cmdline del kernel de captura fijada en ${KDUMP_DEFAULT}"
    echo "    ${cap_cmdline}"
    # Recargar ya el kernel de captura con la cmdline nueva.
    systemctl restart kdump-tools.service >/dev/null 2>&1 \
      && echo "OK: kernel de captura recargado con la cmdline nueva" \
      || echo "AVISO: fallo al recargar kdump-tools; revisa 'systemctl status kdump-tools'"
  fi

  update-grub

  echo ""
  echo "Hecho. HACE FALTA REINICIAR para que se reserve la memoria de captura:"
  echo "  sudo reboot"
  echo "Después:  ./scripts/l6-kdump-setup.sh --status"
  echo "Y para comprobar que de verdad captura, antes de gastar un ciclo de GPU:"
  echo "  sudo ./scripts/l6-kdump-setup.sh --selftest   (¡provoca un panic real!)"
}

# --- disable ------------------------------------------------------------------

do_disable() {
  need_root
  echo "=== Desactivando captura de crash ==="
  rm -f "$GRUB_SNIPPET" && echo "OK: ${GRUB_SNIPPET} eliminado"
  rm -f "$SYSCTL" && echo "OK: ${SYSCTL} eliminado"
  # Los sysctl siguen puestos en caliente hasta el reinicio; bajarlos ya.
  sysctl -q -w kernel.hardlockup_panic=0 2>/dev/null || true
  sysctl -q -w kernel.softlockup_panic=0 2>/dev/null || true
  sysctl -q -w kernel.panic_on_io_nmi=0 2>/dev/null || true
  sysctl -q -w kernel.panic=0 2>/dev/null || true
  echo N >"$POST_NOTIF" 2>/dev/null || true
  if [[ -f "$KDUMP_DEFAULT" ]]; then
    sed -i "/^${CMDLINE_MARK_BEGIN}$/,/^${CMDLINE_MARK_END}$/d" "$KDUMP_DEFAULT"
    echo "OK: cmdline del kernel de captura devuelta al valor por defecto"
  fi
  systemctl disable --now kdump-tools.service >/dev/null 2>&1 || true
  echo "OK: servicio kdump-tools parado y deshabilitado"
  echo "    (el paquete se queda instalado; 'sudo apt-get purge kdump-tools' para quitarlo)"
  update-grub
  echo ""
  echo "Reinicia para liberar la memoria reservada:  sudo reboot"
}

# --- selftest -----------------------------------------------------------------
#
# Un kdump que no se ha probado no es una red de seguridad, es una suposición.
# Esto provoca un panic de verdad: la máquina se cae en el acto y reinicia. Vale
# la pena hacerlo UNA vez, con todo guardado, antes de fiarse de él en el ciclo
# de GPU — que es caro y difícil de reproducir.
do_selftest() {
  need_root
  if [[ "$(cat /sys/kernel/kexec_crash_loaded 2>/dev/null || echo 0)" != "1" ]]; then
    echo "FAIL: el kernel de captura no está cargado; esto solo colgaría la máquina." >&2
    echo "      Revisa ./scripts/l6-kdump-setup.sh --status" >&2
    exit 1
  fi
  if [[ "$(cat "$POST_NOTIF" 2>/dev/null || echo N)" != "Y" ]]; then
    echo "AVISO: crash_kexec_post_notifiers=N. Si el kernel de captura se cuelga," >&2
    echo "       este selftest no dejará NADA, tampoco en pstore. Pasa --enable" >&2
    echo "       primero (o: echo Y | sudo tee ${POST_NOTIF})." >&2
  fi
  echo "Esto va a PROVOCAR UN PANIC AHORA MISMO. La máquina se cae y reinicia."
  echo "Guarda lo que tengas abierto."
  echo ""
  echo "MIRA LA PANTALLA mientras pasa: es el único sitio donde se ve si el kernel"
  echo "de captura arranca. Resultados posibles:"
  echo "  · reinicia sola y aparece un directorio en /var/crash → kdump funciona"
  echo "  · texto en pantalla y se queda ahí → el kernel de captura arrancó pero"
  echo "    falló; lo que ponga es el diagnóstico (fotografíalo)"
  echo "  · pantalla congelada sin una línea → el kexec ni saltó. Al volver, mira"
  echo "    pstore: con crash_kexec_post_notifiers=Y debe tener un registro nuevo"
  read -r -p "Escribe 'si' para continuar: " ans
  [[ "$ans" == "si" ]] || { echo "cancelado"; exit 1; }
  sync
  echo "Provocando panic…"
  echo c >/proc/sysrq-trigger
}

case "$action" in
  --enable)   do_enable ;;
  --disable)  do_disable ;;
  --status)   do_status ;;
  --selftest) do_selftest ;;
  *)
    echo "uso: $0 [--enable|--disable|--status|--selftest]" >&2
    exit 1
    ;;
esac
