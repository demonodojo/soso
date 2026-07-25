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
#   1. kdump  → el dmesg entero del kernel muerto, sacado de /proc/vmcore por el
#      kernel de captura, en /var/crash/<fecha>/dmesg.<fecha>. Lo mejor que se
#      puede sacar HOY en esta máquina; el vmcore completo aquí no es legible y
#      no se pide (ver el cuarto selftest, más abajo).
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
# RESULTADO DEL SEGUNDO SELFTEST, 18:23 DEL 2026-07-25 — el arreglo de pstore
# funciona, kdump sigue sin volcar:
#   · pstore capturó el panic completo (registro 1784996594, 18:23:14) CON el
#     kernel de captura cargado. Termina en `Kernel Offset:`, que es la última
#     línea que imprime el kernel que peta antes de __crash_kexec(). O sea: el
#     canal 2 ya no lo tapa el canal 1. Esto es lo que había que arreglar.
#   · la línea del panic dice `Kdump: loaded`, así que el kernel de captura
#     estaba cargado de verdad.
#   · el kexec SALTÓ: después de esa línea no hay nada y la máquina no reinició
#     a los 120 s de kernel.panic. El control se fue del kernel que peta.
#   · y el kernel de captura murió en el arranque: en /var/crash no apareció ni
#     el directorio con fecha, y ese lo crea kdump-tools-dump.service ANTES de
#     volcar. No llegó a montar /var. Hubo que apagar con el botón.
# Cuidado con leer el silencio de la pantalla como "murió pronto": el kernel
# principal tenía el display programado por i915 y el de captura pinta sobre el
# framebuffer EFI, cuya dirección y stride ya no son lo que el panel escanea.
# Puede arrancar y ser invisible. Lo que acota la muerte temprana es la ausencia
# del directorio, no la pantalla.
# Sospechoso número uno: el IOMMU de Intel, que el kernel de captura hereda
# encendido con las tablas del kernel muerto y con la dGPU en vfio-pci haciendo
# DMA en ese instante. Se prueba con `intel_iommu=off` en KDUMP_CMDLINE, que no
# necesita reiniciar: basta `systemctl restart kdump-tools`.
#
# RESULTADO DEL TERCER SELFTEST, 18:42 DEL 2026-07-25 — el kernel de captura YA
# ARRANCA. Era el IOMMU: con `intel_iommu=off` arrancó, montó /var y escribió.
# Los dos canales, a la vez y sin taparse:
#   · /var/crash/202607251842/ con dmesg (131 KiB) y un dump de 1,3 GiB
#   · pstore, registro 1784997762 (18:42:42)
# Ya no hace falta la rúbrica de "¿arrancó el kernel de captura?": arranca.
# PERO ese dump de 1,3 GiB **no era un volcado completo**, y eso no se supo hasta
# el cuarto selftest. kdump-config lanza
#   makedumpfile $MAKEDUMP_ARGS /proc/vmcore | compress > dump-incomplete
#   ERROR=$?
# y en /bin/sh el `$?` de una tubería es el del ÚLTIMO mandato, o sea el del
# compresor. Si makedumpfile muere —y ahí iba con el OOM killer encima— el
# compresor cierra tan feliz con 0, kdump-config renombra a `dump.<fecha>` y lo
# anuncia como éxito. Un volcado truncado se ve EXACTAMENTE igual que uno bueno.
#
# Lo que quedaba, y era otra cosa: **el que se cuelga es el reinicio de después
# del volcado**, no la captura. El init script de Ubuntu termina así:
#   /etc/init.d/kdump-tools:38   date -R ;
#   /etc/init.d/kdump-tools:39   reboot -f ;
# y la ÚLTIMA línea del journal del kernel de captura es justo la salida de ese
# `date -R` (18:44:03.799). O sea que `reboot -f` se ejecutó y no volvió: pantalla
# a negro, piloto ámbar, sin reinicio, botón. El vmcore ya estaba en disco, así
# que no se pierde nada — pero para pruebas sin nadie delante no sirve.
# `reboot -f` es la llamada reboot(2) normal: notificadores + `device_shutdown()`,
# que recorre todos los dispositivos llamando a su `.shutdown` con la topología
# PCIe a medio morir y `reset_devices` puesto. Ahí se queda. El camino que NO
# pasa por ahí es `emergency_restart()`, o sea sysrq-b, y a eso se llega con el
# envoltorio del paso 6 de do_enable.
# Ojo: `reboot -f` no vacía el journal, así que después de esa línea no hay nada
# escrito ni lo habrá — el silencio del journal no acota dónde se colgó.
#
# Y el kernel de captura iba ASFIXIADO de memoria: 512 MiB para el initrd normal
# de Ubuntu entero más systemd. Mató a los udev-worker en el segundo 2 y dejó
# NetworkManager en bucle de reinicio 24 veces (kdump-tools-dump.service lo
# arrastra con Wants=network-online.target). El volcado tardó 76 s y sobrevivió
# de milagro: el siguiente OOM podía llevarse makedumpfile. Por eso la reserva
# sube a 1 GiB en el paso 3 (de 30 GiB, y esto sí pide reiniciar).
#
# CONSECUENCIA OPERATIVA: con el kernel de captura cargado, el timeout de
# kernel.panic no cubre nada después del kexec — pertenece al kernel que ya no
# está. Todo lo que pase en el kernel de captura tiene que traer su propio plan
# de reinicio: `panic=30` en su cmdline si peta, y el sysrq-b del paso 6 si
# termina bien.
#
# RESULTADO DEL CUARTO SELFTEST, 18:59 DEL 2026-07-25 — con el envoltorio del
# sysrq-b ya puesto y 1280 MiB reservados, la máquina volvió a irse a negro y a no
# reiniciar. Pero esta vez **no estaba colgada: el volcado seguía en marcha**. La
# prueba está en el fichero, no en la pantalla:
#   · /var/crash/202607251859/dump-incomplete, 7,4 GiB, mtime 19:02 — makedumpfile
#     aún escribía en el instante en que se pulsó el botón. Nunca llegó a
#     renombrarse a `dump.<fecha>`, que es lo último que hace kdump-config.
#   · ni un OOM en el journal del kernel de captura, con la wifi asociada y
#     timesyncd sincronizando a los 16 s: estaba perfectamente vivo. El 1 GiB del
#     paso 3 arregló de verdad la asfixia del tercer selftest.
#   · el mensaje del envoltorio ("savecore rc=…") no aparece en ninguna parte,
#     porque savecore todavía no había vuelto. El sysrq-b no llegó a ejecutarse:
#     no es que no funcione, es que nunca le tocó el turno.
# Por qué tardaba tanto: makedumpfile 1.7.5 (abril 2024) no conoce el kernel
# 7.0.0 —lo dice él mismo, "The kernel version is not supported"— así que no puede
# aplicar el filtrado de páginas del `-d 31` y se lleva prácticamente los 30 GiB
# de RAM. Comprimiendo con un solo core (nr_cpus=1) eso son MINUTOS y muchos GiB.
# El tercer selftest "tardó 76 s" nada más que porque murió a la mitad.
# Y lo que remata el asunto: ese vmcore no sirve para nada. No hay
# /usr/lib/debug/boot/vmlinux-7.0.0-28-generic instalado, así que ni makedumpfile
# puede filtrar ni `crash` va a poder abrirlo. Se pagaban varios minutos a ciegas
# y 7 GiB de disco por un fichero que nadie puede leer.
# ARREGLO (paso 6): el envoltorio ya no le pide el vmcore a kdump-config. Saca el
# dmesg de /proc/vmcore —que es lo que hace falta para depurar la GPU, tarda un
# segundo y es lo único legible hoy aquí— y resetea. Para el vmcore completo está
# SOSO_KDUMP_FULL_VMCORE=1, con el precio escrito al lado.
#
# QUINTO SELFTEST, 19:19 DEL 2026-07-25 — LA CADENA ENTERA, SOLA Y SIN BOTÓN:
#   19:19:42  panic en el kernel principal, pstore lo recoge (registro 1784999982)
#   19:19:50  arranca el kernel de captura
#   19:19:59  dmesg sacado de /proc/vmcore: 129914 bytes en
#             /var/crash/202607251919/dmesg.202607251919 — un segundo, no minutos
#   19:19:59  el envoltorio hace sysrq-b
#   19:20:30  la máquina está otra vez arriba
# Cuarenta y ocho segundos de panic a login, sin nadie delante. Es lo que hacía
# falta para poder dejar corriendo pruebas de GPU.
# UNA MINA QUE QUEDA APUNTADA, y no estorbó: el kernel de captura se comió un
# oops en `mtl_dsp_check_ipc_irq` (snd_sof_pci_intel_mtl, el DSP de audio) en el
# hilo irq/136-AudioDS, con `reset_devices` puesto y el DSP a medio inicializar.
# Terminó en "Fixing recursive fault but reboot is needed!", pero en un kthread:
# el envoltorio siguió su camino, volcó y reseteó. Si algún día ese oops se lleva
# por delante la captura, la cura es añadir snd_sof_pci_intel_mtl al
# modprobe.blacklist de KDUMP_CMDLINE — el kernel de captura no necesita audio.
#
# MORALEJA, y ya van cuatro veces: en el kernel de captura la pantalla NO informa
# (pinta sobre el framebuffer EFI mientras el panel sigue con el modo que dejó
# i915) y el journal sólo tiene lo que se alcanzó a vaciar a disco. Lo que dice la
# verdad son los ficheros de /var/crash y sus mtime. "Negro y sin reiniciar" no es
# un diagnóstico: hay que mirar si hay un `dump-incomplete` y cuándo se escribió
# por última vez. Si el mtime avanza, no está colgada — está trabajando.
#
# Uso:
#   sudo ./scripts/l6-kdump-setup.sh --enable    → instalar + configurar, luego reiniciar
#   sudo ./scripts/l6-kdump-setup.sh --disable   → deshacer, luego reiniciar
#   ./scripts/l6-kdump-setup.sh --status         → qué hay puesto ahora (no toca nada)
#
# Precio: reserva ~1 GiB de RAM de forma permanente (de 30 GiB), y a partir de
# ahora un hard lockup **reinicia la máquina** en vez de quedarse colgado. Para
# depurar es lo que se quiere; si molesta, `--disable`.
set -euo pipefail

GRUBD=/etc/default/grub.d
GRUB_SNIPPET="${GRUBD}/soso-l6-kdump.cfg"
SYSCTL=/etc/sysctl.d/99-soso-l6-lockup.conf
KDUMP_DEFAULT=/etc/default/kdump-tools
POST_NOTIF=/sys/module/kernel/parameters/crash_kexec_post_notifiers
# Envoltorio de kdump-config que resetea por sysrq-b al acabar de volcar (paso 6
# de do_enable). Va en /usr/local/sbin porque el kernel de captura monta la raíz
# de verdad, así que desde ahí se ve igual que ahora.
SAVECORE_WRAPPER=/usr/local/sbin/soso-l6-kdump-config
# Lo que debería quedar reservado con el crashkernel= del paso 3, en MiB, para
# distinguir "ya reservado" de "reservado con el valor viejo de 512".
WANT_CRASH_MIB=1024
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
  local crash_mib=$((crash_size / 1024 / 1024))
  if [[ "$crash_size" -le 0 ]]; then
    echo "FALTA memoria de crash reservada (kexec_crash_size = 0)"
  elif [[ "$crash_mib" -lt "$WANT_CRASH_MIB" ]]; then
    # 512 MiB es el valor de la tabla de Ubuntu, y con él el kernel de captura
    # arranca en OOM permanente: volcó, pero matando procesos por el camino.
    echo "AVISO memoria reservada: ${crash_mib} MiB, se quieren ${WANT_CRASH_MIB}"
    echo "      (con 512 el kernel de captura vuelca en OOM permanente; reinicia"
    echo "       para coger el crashkernel= nuevo)"
  else
    echo "OK    memoria reservada: ${crash_mib} MiB"
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
    # Ojo con prometer demasiado: este timeout solo cubre "el kexec no saltó".
    # Si salta y el kernel de captura se cuelga, el timeout se fue con el kernel
    # que petó y la máquina se queda muerta pidiendo botón (2026-07-25 18:23).
    echo "OK    kernel.panic=${pt} — si el kexec NO salta, reinicia en vez de colgarse"
    echo "      (si salta y el kernel de captura se cuelga, no hay timeout: botón)"
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

  # El reinicio de después del volcado. Sin esto, un panic deja el vmcore escrito
  # y la máquina colgada pidiendo botón (2026-07-25 18:44).
  local wrapper_cfg
  wrapper_cfg=$(grep -m1 '^KDUMP_SCRIPT=' "$KDUMP_DEFAULT" 2>/dev/null | cut -d= -f2- || true)
  if [[ -z "$wrapper_cfg" ]]; then
    echo "FALTA envoltorio de savecore: al acabar el volcado, el init script hace"
    echo "      'reboot -f' y aquí eso se cuelga en device_shutdown(). El vmcore se"
    echo "      escribe, pero hay que apagar con el botón (--enable lo pone)"
  elif [[ -x "$wrapper_cfg" ]]; then
    echo "OK    envoltorio de savecore: ${wrapper_cfg} (resetea por sysrq-b al volcar)"
  else
    # Peligroso de verdad: el init script llama a KDUMP_SCRIPT también para
    # 'load', así que si no existe, kdump no se carga siquiera.
    echo "FALLO KDUMP_SCRIPT=${wrapper_cfg} no es ejecutable — kdump no puede ni"
    echo "      cargar el kernel de captura. Arreglar: --enable (o --disable)"
  fi

  local svc
  svc=$(systemctl is-enabled kdump-tools.service 2>/dev/null || true)
  echo "      servicio kdump-tools: ${svc:-(no instalado)}"

  # Los registros NO se leen en /sys/fs/pstore: systemd-pstore.service los saca
  # de ahí en cada arranque (deja el directorio vacío, que engaña) y los archiva
  # en /var/lib/systemd/pstore/<epoch>/001/, con un dmesg.txt ya reensamblado.
  # En los chunks sueltos la parte 01 es la COLA del log y la última es el
  # principio; dmesg.txt viene ya en orden.
  echo "      pstore (segundo canal, independiente de kdump):"
  echo "        se archivan en /var/lib/systemd/pstore/<epoch>/001/dmesg.txt"
  if [[ -d /var/lib/systemd/pstore ]] && [[ -n "$(ls -A /var/lib/systemd/pstore 2>/dev/null)" ]]; then
    local rec
    for rec in $(ls -1 /var/lib/systemd/pstore); do
      if [[ "$rec" =~ ^[0-9]+$ ]]; then
        echo "        registro ${rec} — $(date -d "@${rec}" '+%F %T' 2>/dev/null || echo '?')"
      else
        echo "        registro ${rec}"
      fi
    done
  else
    echo "        (vacío)"
  fi

  # Qué se vuelca. El vmcore completo aquí es una trampa: minutos a ciegas por un
  # fichero ilegible, y esos minutos ya se confundieron una vez con un cuelgue.
  local full_mode
  full_mode=$(grep -m1 '^SOSO_KDUMP_FULL_VMCORE=' "$KDUMP_DEFAULT" 2>/dev/null | cut -d= -f2- || true)
  if [[ "$full_mode" == "1" ]]; then
    echo "AVISO modo de volcado: VMCORE COMPLETO (SOSO_KDUMP_FULL_VMCORE=1)"
    echo "      son varios minutos SIN señal en pantalla y ~7 GiB por panic. Y para"
    echo "      poder abrirlo hace falta linux-image-$(uname -r)-dbgsym,"
    local dbg=/usr/lib/debug/boot/vmlinux-$(uname -r)
    [[ -e "$dbg" ]] && echo "      que está instalado ($dbg)." \
                    || echo "      que NO está instalado: el fichero no lo va a leer nadie."
  else
    echo "OK    modo de volcado: sólo el dmesg (un segundo). El vmcore completo no"
    echo "      se pide: makedumpfile $(makedumpfile -v 2>/dev/null | head -1 | grep -o 'version [0-9.]*' || echo '?') no conoce el kernel $(uname -r),"
    echo "      así que no filtra páginas y el resultado no hay quien lo abra"
  fi

  # kdump escribe directorios <fecha>/. Los ficheros .crash sueltos que ya hay ahí
  # son de apport (cuelgues de programas de usuario) y no pintan nada.
  echo "      lo capturado en /var/crash:"
  local dumps
  dumps=$(find /var/crash -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort || true)
  if [[ -z "$dumps" ]]; then
    echo "        (ninguno — los .crash sueltos de ahí son de apport, no de kdump)"
  else
    local d f
    while read -r d; do
      [[ -n "$d" ]] || continue
      echo "        ${d}"
      for f in "$d"/*; do
        [[ -e "$f" ]] || continue
        # 'dump-incomplete' = makedumpfile no llegó al final; kdump-config sólo
        # renombra a dump.<fecha> cuando termina. Si esto está aquí, o el volcado
        # se cortó a botón (2026-07-25 18:59) o murió por el camino. En ninguno de
        # los dos casos vale nada, y ocupa GiB.
        if [[ "$(basename "$f")" == dump-incomplete* ]]; then
          echo "          $(basename "$f")  $(du -h "$f" 2>/dev/null | cut -f1)  ← TRUNCADO"
          echo "            (última escritura: $(date -r "$f" '+%F %T' 2>/dev/null || echo '?'))"
          echo "            no vale para nada y ocupa sitio:  sudo rm -rf ${d}"
        else
          echo "          $(basename "$f")  $(du -h "$f" 2>/dev/null | cut -f1)"
        fi
      done
    done <<<"$dumps"
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
#
# 1024M para el tramo 4G-32G, que es el de esta máquina (30 GiB), en vez de los
# 512M de la tabla por defecto de Ubuntu: con 512M el kernel de captura del
# selftest de las 18:42 del 2026-07-25 arrancó en OOM permanente —udev-worker
# muertos en el segundo 2, NetworkManager en bucle de reinicio 24 veces— y el
# volcado salió por los pelos. El initrd que carga es el normal de Ubuntu,
# descomprimido en RAM, más systemd entero: 512M no dan.
GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT crashkernel=2G-4G:320M,4G-32G:1024M,32G-64G:1024M,64G-:2048M crash_kexec_post_notifiers"
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
  #      intel_iommu=off → AÑADIDO tras el selftest de las 18:23 del 2026-07-25,
  #                   donde el kernel de captura murió antes de montar /var. Es
  #                   el sospechoso número uno: lo hereda encendido, con las
  #                   tablas de traducción del kernel muerto, y con la dGPU en
  #                   vfio-pci haciendo DMA en el instante del panic. Para
  #                   volcar memoria no se necesita traducción ninguna.
  #                   Es LA variable que se está probando: si con esto aparece
  #                   directorio en /var/crash, era el IOMMU.
  #      panic=30   → si el kernel de captura peta, que reinicie en vez de dejar
  #                   la máquina muerta pidiendo botón. kernel.panic del kernel
  #                   principal no llega aquí: murió con él. No influye en que
  #                   el volcado aparezca o no, solo en tener que levantarse.
  local root_spec
  root_spec=$(grep -o 'root=[^ ]*' /proc/cmdline || true)
  if [[ -z "$root_spec" ]]; then
    echo "AVISO: no encuentro root= en /proc/cmdline; dejo KDUMP_CMDLINE sin tocar" >&2
  else
    local cap_cmdline="${root_spec} ro nomodeset loglevel=7 console=tty0"
    cap_cmdline+=" intel_iommu=off panic=30"
    cap_cmdline+=" modprobe.blacklist=vfio_pci,vfio_iommu_type1,nvidia,nvidia_drm,nvidia_modeset,nvidia_uvm,nouveau"
    # 6) QUÉ se vuelca, y que el kernel de captura REINICIE al acabar.
    #    Las dos cosas van juntas porque salen del mismo sitio: un envoltorio de
    #    kdump-config. Se puede sustituir desde aquí porque el init script fija
    #    KDUMP_SCRIPT en su línea 22 ANTES de leer este fichero de defaults en la
    #    24, así que nuestro valor gana.
    #
    #    a) Reinicio: el init script acaba en `reboot -f`, y aquí eso se cuelga —
    #       pantalla negra, piloto ámbar y botón (selftest de las 18:42 del
    #       2026-07-25; la última línea del journal es la del `date -R` que va
    #       justo antes). reboot(2) pasa por los notificadores y por
    #       device_shutdown(), que llama al .shutdown de cada dispositivo con el
    #       PCIe a medio morir y reset_devices puesto. emergency_restart()
    #       —sysrq-b— no pasa por ahí: resetea la máquina y punto. El envoltorio
    #       sincroniza él mismo, porque sysrq-b no baja los sistemas de ficheros.
    #    b) Contenido: sólo el dmesg. El vmcore completo son minutos a ciegas y
    #       ~7 GiB por panic, y en esta máquina no hay quien lo abra (makedumpfile
    #       1.7.5 no conoce el kernel 7.0.0 y no hay vmlinux con símbolos). Esos
    #       minutos son los que el 2026-07-25 a las 18:59 se confundieron con un
    #       cuelgue y se cortaron a botón. El dmesg sale en un segundo y es lo que
    #       de verdad se necesita. SOSO_KDUMP_FULL_VMCORE=1 recupera el vmcore
    #       para quien lo quiera, con su precio.
    cat >"$SAVECORE_WRAPPER" <<'EOF'
#!/bin/sh
# soso L6 — envoltorio de /usr/sbin/kdump-config.
# Puesto por scripts/l6-kdump-setup.sh --enable; se quita con --disable.
#
# Corre DENTRO del kernel de captura, con un solo core (nr_cpus=1) y sin nadie
# delante. Cambia dos cosas de lo que haría kdump-config:
#
#   1. VUELCA SÓLO EL DMESG. El vmcore completo en esta máquina cuesta varios
#      minutos —30 GiB de RAM sin filtrar, comprimidos por un core— y no lo puede
#      abrir nadie: makedumpfile 1.7.5 no conoce el kernel 7.0.0 ("The kernel
#      version is not supported", así que el `-d 31` no filtra) y no hay
#      vmlinux con símbolos instalado. El 2026-07-25 a las 18:59 esos minutos a
#      ciegas se confundieron con un cuelgue y se apagó a botón a mitad del
#      volcado. El dmesg, en cambio, sale en un segundo y es exactamente lo que
#      se necesita para depurar el bring-up de la GPU.
#      Si algún día hace falta el vmcore de verdad: SOSO_KDUMP_FULL_VMCORE=1 en
#      /etc/default/kdump-tools (y a esperar, ver más abajo).
#
#   2. RESETEA POR sysrq-b al terminar, porque el `reboot -f` con el que acaba
#      /etc/init.d/kdump-tools se cuelga aquí en device_shutdown(): recorre todos
#      los dispositivos con el PCIe a medio morir y reset_devices puesto.
#      emergency_restart() no pasa por ahí.
#
# Todo lo que no sea savecore (load, unload, status…) se pasa tal cual.
REAL=/usr/sbin/kdump-config
[ "${1:-}" = savecore ] || exec "$REAL" "$@"

: "${KDUMP_COREDIR:=/var/crash}"

# Al kmsg y al journal a la vez: el kmsg va a console=tty0, que en el kernel de
# captura puede no verse (framebuffer EFI contra el modo que dejó i915); el
# journal se escribe en la raíz de verdad y sí se puede leer al volver, con
# `journalctl -b -1 -t soso-l6-kdump`.
say() {
  echo "soso L6 kdump: $*" >/dev/kmsg 2>/dev/null || true
  logger -t soso-l6-kdump "$*" 2>/dev/null || true
}

# La raíz llega 'ro' en la cmdline del kernel de captura. Hoy acaba rw de todas
# formas, pero sin esto un día que no lo hiciera el volcado se perdería en
# silencio, que es el fallo que más caro sale aquí.
[ -w "$KDUMP_COREDIR" ] || mount -o remount,rw / 2>/dev/null || true

# Mismo esquema de nombres que kdump-config, para que --status y el ojo humano
# vean una sola convención en /var/crash.
stamp=$(date +%Y%m%d%H%M)
dir="$KDUMP_COREDIR/$stamp"
n=1
while [ -e "$dir/dmesg.$stamp" ]; do
  n=$((n + 1))
  dir="$KDUMP_COREDIR/$stamp-$n"
done
mkdir -p "$dir" || say "no puedo crear $dir"

# Sin tubería a propósito: así `$?` es el de makedumpfile y no el de un compresor
# que cierra con 0 aunque el volcado se haya truncado. Ese detalle es el que hizo
# pasar por bueno el volcado a medias del 2026-07-25 18:42.
say "extrayendo el dmesg de /proc/vmcore en $dir"
if makedumpfile --dump-dmesg /proc/vmcore "$dir/dmesg.$stamp" >/dev/null 2>&1; then
  rc=0
  say "dmesg guardado en $dir/dmesg.$stamp ($(wc -c <"$dir/dmesg.$stamp" 2>/dev/null) bytes)"
else
  rc=1
  say "FALLO extrayendo el dmesg; queda pstore como segundo canal"
fi

# El vmcore completo sólo si se ha pedido a mano, y avisando de lo que va a pasar:
# minutos sin una señal en pantalla. Es justo el tramo en el que la máquina parece
# colgada y no lo está.
if [ "${SOSO_KDUMP_FULL_VMCORE:-0}" = 1 ]; then
  say "SOSO_KDUMP_FULL_VMCORE=1: vmcore completo. Son MINUTOS y ~7 GiB, y la"
  say "pantalla no va a decir nada. NO apagues: mira el mtime de dump-incomplete."
  "$REAL" savecore
  say "kdump-config savecore devolvió $?"
  # Ese código de salida no vale para saber si el volcado está entero (viene del
  # compresor de la tubería). Lo que sí lo dice es que no quede ningún
  # dump-incomplete: kdump-config lo renombra a dump.<fecha> sólo al final.
  left=$(find "$KDUMP_COREDIR" -maxdepth 2 -name 'dump-incomplete*' 2>/dev/null | tr '\n' ' ')
  [ -z "$left" ] || say "OJO: volcado TRUNCADO, se queda $left"
fi

say "sincronizando y reset de emergencia (sysrq-b)"
sync
if [ -w /proc/sysrq-trigger ]; then
  # Escribir en /proc/sysrq-trigger salta la máscara de kernel.sysrq (aquí 176,
  # que no incluiría el bit de dump), así que no hay que tocar el sysctl.
  echo s >/proc/sysrq-trigger 2>/dev/null || true   # sync del kernel
  echo u >/proc/sysrq-trigger 2>/dev/null || true   # remontar en solo lectura
  echo b >/proc/sysrq-trigger 2>/dev/null || true   # emergency_restart()
fi

# Si sysrq no estuviera disponible, devolver el estado del dmesg y dejar que el
# init script intente su reboot -f: peor, pero es lo que había antes.
exit $rc
EOF
    chmod 755 "$SAVECORE_WRAPPER"
    echo "OK: ${SAVECORE_WRAPPER}"

    sed -i "/^${CMDLINE_MARK_BEGIN}$/,/^${CMDLINE_MARK_END}$/d" "$KDUMP_DEFAULT"
    {
      echo "$CMDLINE_MARK_BEGIN"
      echo "# Puesto por scripts/l6-kdump-setup.sh --enable; se quita con --disable."
      echo "KDUMP_CMDLINE=\"${cap_cmdline}\""
      echo "# Envoltorio de kdump-config: vuelca sólo el dmesg y resetea por sysrq-b,"
      echo "# porque el 'reboot -f' del init script se cuelga en device_shutdown()."
      echo "# Funciona porque el init script fija KDUMP_SCRIPT antes de leer esto."
      echo "KDUMP_SCRIPT=${SAVECORE_WRAPPER}"
      echo "# Ponlo a 1 para volcar además el vmcore completo. Precio medido el"
      echo "# 2026-07-25: varios minutos con un solo core y ~7 GiB por panic, sin"
      echo "# ninguna señal en pantalla mientras dura — y hoy ni makedumpfile ni"
      echo "# crash pueden abrir el resultado, porque falta el paquete"
      echo "# linux-image-\$(uname -r)-dbgsym. Actívalo sólo si vas a instalarlo."
      echo "SOSO_KDUMP_FULL_VMCORE=0"
      echo "# Cuántos directorios de /var/crash conserva kdump-config al volcar el"
      echo "# vmcore completo. Sin esto (0 = sin límite) unos pocos panics llenan el"
      echo "# disco a 7 GiB por cabeza."
      echo "KDUMP_NUM_DUMPS=5"
      echo "$CMDLINE_MARK_END"
    } >>"$KDUMP_DEFAULT"
    echo "OK: cmdline del kernel de captura fijada en ${KDUMP_DEFAULT}"
    echo "    ${cap_cmdline}"
    # Recargar ya el kernel de captura con la cmdline nueva. Esto además prueba
    # el envoltorio: el 'load' pasa por él.
    systemctl restart kdump-tools.service >/dev/null 2>&1 \
      && echo "OK: kernel de captura recargado con la cmdline nueva" \
      || echo "AVISO: fallo al recargar kdump-tools; revisa 'systemctl status kdump-tools'"
  fi

  update-grub

  # Lo único que necesita el arranque es la reserva de memoria. Si ya está hecha
  # (re-ejecución para cambiar la cmdline del kernel de captura, por ejemplo),
  # no hay nada que esperar: los sysctl van en caliente y kdump-tools ya se ha
  # recargado unas líneas más arriba.
  echo ""
  local now_mib
  now_mib=$(( $(cat /sys/kernel/kexec_crash_size 2>/dev/null || echo 0) / 1024 / 1024 ))
  if [[ "$now_mib" -ge "$WANT_CRASH_MIB" ]]; then
    echo "Hecho, y SIN REINICIAR: la memoria de captura ya estaba reservada de un"
    echo "arranque anterior, y todo lo demás se aplica en caliente."
  elif [[ "$now_mib" -gt 0 ]]; then
    echo "Hecho, y lo que hace falta ya está en caliente. Pero la reserva actual"
    echo "es de ${now_mib} MiB y se quieren ${WANT_CRASH_MIB}: con 512 el kernel de captura"
    echo "vuelca con el OOM killer encima. Para subirla:  sudo reboot"
  else
    echo "Hecho. HACE FALTA REINICIAR para que se reserve la memoria de captura:"
    echo "  sudo reboot"
  fi
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
    # Primero la referencia y luego el fichero: al revés, un 'kdump-config unload'
    # por el medio se quedaría sin script que llamar.
    sed -i "/^${CMDLINE_MARK_BEGIN}$/,/^${CMDLINE_MARK_END}$/d" "$KDUMP_DEFAULT"
    echo "OK: cmdline y KDUMP_SCRIPT del kernel de captura devueltos a su valor por defecto"
  fi
  if [[ -e "$SAVECORE_WRAPPER" ]]; then
    rm -f "$SAVECORE_WRAPPER" && echo "OK: ${SAVECORE_WRAPPER} eliminado"
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
  local pt_hint
  pt_hint=$(sysctl -n kernel.panic 2>/dev/null || echo 120)
  echo "Esto va a PROVOCAR UN PANIC AHORA MISMO. La máquina se cae y reinicia."
  echo "Guarda lo que tengas abierto."
  echo ""
  # La rúbrica de antes decía que ver texto de panic en pantalla significaba que
  # el kernel de captura había arrancado. Con crash_kexec_post_notifiers=Y eso
  # es falso: ese texto lo imprime el kernel principal ANTES de saltar. Y el
  # silencio tampoco prueba nada, porque el kernel de captura pinta sobre el
  # framebuffer EFI mientras el panel sigue con lo que programó i915: puede
  # estar arrancando y no verse. Lo que de verdad distingue es esto:
  echo "Lo que hay que mirar AL VOLVER, no en la pantalla (con"
  echo "crash_kexec_post_notifiers=Y la pantalla ya no distingue nada: el texto"
  echo "del panic lo imprime el kernel principal antes de saltar al de captura,"
  echo "y el de captura puede arrancar sin que se vea una línea)."
  echo ""
  echo "  · reinició SOLA en cuestión de segundos y hay un directorio con fecha en"
  echo "    /var/crash con un dmesg.<fecha> dentro"
  echo "        → camino completo: capturó y reseteó por sysrq-b. Es lo esperado."
  echo "  · se quedó en negro y no reinicia"
  echo "        → NO des por hecho que está colgada, que es el error que se cometió"
  echo "          el 2026-07-25 a las 18:59: el volcado seguía en marcha (7,4 GiB y"
  echo "          contando) y se cortó a botón. ESPERA, y al volver mira si quedó un"
  echo "          dump-incomplete: --status lo marca como TRUNCADO."
  echo "        → sólo si el dmesg ya está escrito y aun así no reinició es el reset"
  echo "          lo que se cuelga: entonces prueba reboot=pci o reboot=triple en"
  echo "          KDUMP_CMDLINE (ni sysrq-b resetearía esta máquina desde el kexec)"
  echo "  · directorio con fecha pero sin dmesg dentro"
  echo "        → el kernel de captura arrancó y montó /var, y falló al volcar:"
  echo "          journalctl -b -1 -t soso-l6-kdump   (y -u kdump-tools-dump)"
  echo "  · NI el directorio, y la máquina se quedó muerta pidiendo botón"
  echo "        → el kexec saltó pero el kernel de captura murió antes de montar"
  echo "          /var (pasó el 2026-07-25 18:23, era el IOMMU). Que no reinicie a"
  echo "          los ${pt_hint} s lo confirma: ese timeout murió con el kernel que petó"
  echo "  · reinició sola a los ${pt_hint} s y no hay directorio"
  echo "        → el kexec no llegó a saltar; el panic siguió su curso normal"
  echo ""
  echo "En todos los casos pstore debe tener un registro nuevo — es el canal que no"
  echo "depende de que el kernel de captura llegue a ninguna parte:"
  echo "  ./scripts/l6-kdump-setup.sh --status   (lista los registros con fecha)"
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
